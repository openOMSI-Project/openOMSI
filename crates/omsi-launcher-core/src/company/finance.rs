//! Loans: the bank lends against a fixed amount, the fleet's value (and the bus a loan buys)
//! and what the company earns, at the difficulty's interest (none on Easy) less what its level
//! saves, paid back in equal monthly rates. A loan is taken by signing its contract
//! (`LoanContract`); paying it back early costs a fee on Realistic and Hard.

use super::dates;
use super::economy;
use super::levels;
use super::market;
use super::model::{BookingKind, Cents, Company, Difficulty, Loan};
use serde::{Deserialize, Serialize};

/// What the owned fleet is worth.
pub fn fleet_value(c: &Company) -> Cents {
    c.fleet.iter().map(|v| market::value_of(c, v)).sum()
}

/// What the bank's credit is made of: a fixed part for a company of the difficulty, a share
/// of what the buses are worth (with what a loan buys), six months of what the company
/// earns, less what it owes already; and the interest it asks, less the level's discount.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Credit {
    pub base: Cents,
    pub fleet_value: Cents,
    pub share: f64,
    /// The average monthly result of the last three months (none when it lost money).
    pub income: Cents,
    pub limit: Cents,
    pub debt: Cents,
    pub left: Cents,
    /// Yearly interest, after the level's discount (`discount`).
    pub rate: f64,
    pub discount: f64,
}

/// How many months of the result the bank counts.
pub const INCOME_MONTHS: i64 = 6;

/// The average monthly result of the three months before the company's month (0 when it lost
/// money; months without bookings count as nothing earned).
pub fn monthly_income(c: &Company) -> Cents {
    let mut first = format!("{}-01", dates::month_of(&c.date));
    let mut sum = 0;
    for _ in 0..3 {
        let prev = dates::add(&first, -1);
        sum += c.month(&dates::month_of(&prev)).result();
        first = format!("{}-01", dates::month_of(&prev));
    }
    (sum / 3).max(0)
}

/// The yearly interest the bank asks of the company: the difficulty's, less what its level
/// saves (`levels::loan_discount`).
pub fn rate_of(c: &Company) -> f64 {
    (economy::rules(c.difficulty).loan_rate - levels::loan_discount(c)).max(0.0)
}

/// The company's credit, with `collateral` (the value of what a loan buys).
pub fn credit(c: &Company, collateral: Cents) -> Credit {
    let r = economy::rules(c.difficulty);
    let base = (r.credit_base as f64 * c.price_index).round() as Cents;
    let fleet_value = fleet_value(c) + collateral;
    let income = monthly_income(c);
    let limit = base + (r.credit_share * fleet_value as f64).round() as Cents + INCOME_MONTHS * income;
    let debt = c.debt();
    let rate = rate_of(c);
    Credit { base, fleet_value, share: r.credit_share, income, limit, debt, left: (limit - debt).max(0), rate, discount: (r.loan_rate - rate).max(0.0) }
}

/// How much the bank lends in all, with `collateral` (the value of what the loan buys).
pub fn credit_limit(c: &Company, collateral: Cents) -> Cents {
    credit(c, collateral).limit
}

/// How much more the bank lends.
pub fn credit_left(c: &Company, collateral: Cents) -> Cents {
    credit(c, collateral).left
}

/// How much room is left: plenty, little (under a quarter of the limit), or none (less than
/// the smallest loan).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Room {
    Plenty,
    Little,
    None,
}

/// The smallest loan the bank gives.
pub const SMALLEST_LOAN: Cents = 10_000_00;

pub fn room(cr: &Credit) -> Room {
    if cr.left < SMALLEST_LOAN {
        Room::None
    } else if (cr.left as f64) < cr.limit as f64 * 0.25 {
        Room::Little
    } else {
        Room::Plenty
    }
}

/// What a loan of `amount` costs a month, and over how many months (the difficulty's term).
pub fn loan_terms(c: &Company, amount: Cents) -> (Cents, u32, f64) {
    let r = economy::rules(c.difficulty);
    let rate = rate_of(c);
    (economy::annuity(amount, rate, r.loan_months), r.loan_months, rate)
}

/// The terms the bank offers, in months: up to the difficulty's longest.
pub fn terms_offered(c: &Company) -> Vec<u32> {
    let longest = economy::rules(c.difficulty).loan_months;
    [12, 24, 36, 48, 60, 72, 84, 96].into_iter().filter(|m| *m <= longest).collect()
}

/// One month of a loan's repayment: the rate, its interest and principal, and what is left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instalment {
    pub month: u32,
    pub payment: Cents,
    pub interest: Cents,
    pub principal: Cents,
    pub remaining: Cents,
}

/// A loan's repayment month by month, as `pay_rates` books it.
pub fn schedule(amount: Cents, rate: f64, months: u32) -> Vec<Instalment> {
    let monthly = economy::annuity(amount, rate, months);
    let mut left = amount;
    let mut out = Vec::new();
    for m in 1..=months {
        if left <= 0 {
            break;
        }
        let interest = (left as f64 * rate / 12.0).round() as Cents;
        let principal = if m == months { left } else { (monthly - interest).clamp(0, left) };
        left -= principal;
        out.push(Instalment { month: m, payment: interest + principal, interest, principal, remaining: left });
    }
    out
}

/// The fee for paying a loan back early, of the amount paid back.
pub fn early_fee_rate(d: Difficulty) -> f64 {
    match d {
        Difficulty::Easy => 0.0,
        Difficulty::Realistic => 0.01,
        Difficulty::Hard => 0.02,
    }
}

/// The fee for paying `amount` back early.
pub fn early_fee(c: &Company, amount: Cents) -> Cents {
    (amount as f64 * early_fee_rate(c.difficulty)).round() as Cents
}

/// A loan contract: who lends to whom, how much, at what interest, over how long, the rates,
/// what paying back early costs and what secures it - and the signature that makes it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct LoanContract {
    /// Its number: the loan's id (given when it is signed).
    pub no: u32,
    pub lender: String,
    pub borrower: String,
    pub amount: Cents,
    pub rate: f64,
    pub months: u32,
    pub monthly: Cents,
    /// What is paid back in all.
    pub total: Cents,
    /// The day of the first rate (the month's end).
    pub first_rate: String,
    pub purpose: String,
    /// What the buses that secure it are worth (with what it buys).
    pub security: Cents,
    pub early_fee: f64,
    pub signed_by: String,
    #[serde(default)]
    pub strokes: Vec<Vec<[f32; 2]>>,
    pub signed_at: String,
}

impl LoanContract {
    pub fn interest(&self) -> Cents {
        self.total - self.amount
    }

    pub fn is_signed(&self) -> bool {
        !self.signed_by.trim().is_empty() || self.strokes.iter().any(|s| s.len() > 1)
    }
}

/// The bank (made up).
pub const BANK: &str = "Verkehrs- und Gewerbebank";

/// The contract for a loan of `amount` over `months` (`collateral`: the value of what it buys).
pub fn draft_loan(c: &Company, amount: Cents, months: u32, purpose: &str, collateral: Cents) -> LoanContract {
    let rate = rate_of(c);
    let months = months.max(1);
    let monthly = economy::annuity(amount, rate, months);
    let total = schedule(amount, rate, months).iter().map(|i| i.payment).sum();
    let mut first = c.date.clone();
    while !dates::last_of_month(&first) && dates::parse(&first).is_some() {
        first = dates::add(&first, 1);
    }
    LoanContract {
        lender: BANK.to_string(),
        borrower: c.name.clone(),
        amount,
        rate,
        months,
        monthly,
        total,
        first_rate: first,
        purpose: purpose.to_string(),
        security: fleet_value(c) + collateral,
        early_fee: early_fee_rate(c.difficulty),
        ..Default::default()
    }
}

/// Sign a loan contract: the money is in the cash at once, the rates follow at every month's
/// end. Returns the loan's id.
pub fn sign_loan(c: &mut Company, k: &LoanContract, collateral: Cents) -> Result<u32, &'static str> {
    if !k.is_signed() {
        return Err("Sign the contract first.");
    }
    if k.amount <= 0 {
        return Err("Choose an amount.");
    }
    if k.amount > credit_left(c, collateral) {
        return Err("The bank does not lend that much.");
    }
    c.counters.loan += 1;
    let id = c.counters.loan;
    let purpose = if k.purpose.trim().is_empty() { "Loan".to_string() } else { k.purpose.clone() };
    c.loans.push(Loan { id, taken: c.date.clone(), principal: k.amount, remaining: k.amount, rate: k.rate, monthly: k.monthly, months_left: k.months, purpose: purpose.clone() });
    c.book(BookingKind::Loan, k.amount, purpose, false);
    let mut k = k.clone();
    k.no = id;
    k.signed_at = c.date.clone();
    c.dealer.loan_contracts.push(k);
    if c.dealer.loan_contracts.len() > 60 {
        c.dealer.loan_contracts.remove(0);
    }
    Ok(id)
}

/// Sign a loan and do what it pays for, all or nothing: when `f` refuses, the loan is not
/// taken either.
pub fn with_loan<T>(c: &mut Company, k: &LoanContract, collateral: Cents, f: impl FnOnce(&mut Company) -> Result<T, &'static str>) -> Result<(u32, T), &'static str> {
    let mut tmp = c.clone();
    let id = sign_loan(&mut tmp, k, collateral)?;
    let out = f(&mut tmp)?;
    *c = tmp;
    Ok((id, out))
}

/// Take a loan: the money is in the cash at once. Returns its id. (The pages sign a contract
/// for it: `sign_loan`.)
pub fn take_loan(c: &mut Company, amount: Cents, purpose: &str, collateral: Cents) -> Result<u32, &'static str> {
    if amount <= 0 {
        return Err("Choose an amount.");
    }
    if amount > credit_left(c, collateral) {
        return Err("The bank does not lend that much.");
    }
    let (monthly, months, rate) = loan_terms(c, amount);
    c.counters.loan += 1;
    let id = c.counters.loan;
    c.loans.push(Loan { id, taken: c.date.clone(), principal: amount, remaining: amount, rate, monthly, months_left: months, purpose: purpose.to_string() });
    c.book(BookingKind::Loan, amount, purpose.to_string(), false);
    Ok(id)
}

/// Pay back `amount` of a loan early, from the cash, with the early repayment fee (the rate
/// stays, the term shortens).
pub fn repay(c: &mut Company, id: u32, amount: Cents) -> Result<(), &'static str> {
    let Some(i) = c.loans.iter().position(|l| l.id == id) else { return Err("There is no such loan.") };
    let amount = amount.min(c.loans[i].remaining);
    if amount <= 0 {
        return Err("Choose an amount.");
    }
    let fee = early_fee(c, amount);
    if c.cash < amount + fee {
        return Err("Not enough cash.");
    }
    c.loans[i].remaining -= amount;
    let purpose = c.loans[i].purpose.clone();
    c.book(BookingKind::Repayment, -amount, purpose.clone(), false);
    c.book(BookingKind::Interest, -fee, format!("{purpose} (early repayment fee)"), false);
    if c.loans[i].remaining <= 0 {
        c.loans.remove(i);
    }
    Ok(())
}

/// The month's rates of every loan: interest on what is left, the rest repays it. Returns
/// the purposes of the loans paid off.
pub fn pay_rates(c: &mut Company) -> Vec<String> {
    let mut done = Vec::new();
    let loans = std::mem::take(&mut c.loans);
    let mut kept = Vec::new();
    for mut l in loans {
        let interest = (l.remaining as f64 * l.rate / 12.0).round() as Cents;
        let principal = (l.monthly - interest).clamp(0, l.remaining);
        // (the last rate pays what is left, whatever the rounding left over)
        let principal = if l.months_left <= 1 { l.remaining } else { principal };
        c.book(BookingKind::Interest, -interest, l.purpose.clone(), false);
        c.book(BookingKind::Repayment, -principal, l.purpose.clone(), false);
        l.remaining -= principal;
        l.months_left = l.months_left.saturating_sub(1);
        if l.remaining <= 0 {
            done.push(l.purpose.clone());
        } else {
            kept.push(l);
        }
    }
    c.loans = kept;
    done
}

#[cfg(test)]
mod tests {
    use super::super::{found, Founding};
    use super::*;
    use crate::company::model::Difficulty;

    #[test]
    fn a_loan_is_paid_off_in_its_months() {
        let mut c = found(&Founding { name: "Bank".into(), difficulty: Difficulty::Realistic, ..Default::default() }, "Luc");
        let cash = c.cash;
        assert!(take_loan(&mut c, 10_000_000_00, "too much", 0).is_err());
        let id = take_loan(&mut c, 100_000_00, "bus", 0).unwrap();
        assert_eq!(c.cash, cash + 100_000_00);
        assert_eq!(c.debt(), 100_000_00);
        let mut paid = 0;
        let mut interest = 0;
        for _ in 0..72 {
            let before = c.cash;
            let m = c.month(&super::super::dates::month_of(&c.date)).get(BookingKind::Interest);
            pay_rates(&mut c);
            interest = c.month(&super::super::dates::month_of(&c.date)).get(BookingKind::Interest) - m + interest;
            paid += before - c.cash;
        }
        assert!(c.loans.is_empty(), "{:?}", c.loans);
        // the rates came to the principal and the interest on it
        assert!(paid > 100_000_00 && paid < 115_000_00, "{paid}");
        assert_eq!(paid, 100_000_00 - interest);
        // an early repayment shortens it
        let id2 = take_loan(&mut c, 50_000_00, "more", 0).unwrap();
        repay(&mut c, id2, 20_000_00).unwrap();
        assert_eq!(c.debt(), 30_000_00);
        repay(&mut c, id2, 99_000_00).unwrap();
        assert!(c.loans.is_empty());
        let _ = id;
        // Easy lends without interest
        let mut e = found(&Founding { name: "Easy".into(), difficulty: Difficulty::Easy, ..Default::default() }, "Luc");
        take_loan(&mut e, 96_000_00, "x", 0).unwrap();
        assert_eq!(e.loans[0].monthly, 1_000_00);
    }

    #[test]
    fn the_annuity_pays_the_loan_off_to_the_cent() {
        // 100,000 at 6 % over 12 months: the textbook rate is 8,606.64
        assert_eq!(economy::annuity(100_000_00, 0.06, 12), 8_606_64);
        assert_eq!(economy::annuity(120_000_00, 0.0, 12), 10_000_00);
        for (amount, rate, months) in [(100_000_00, 0.06, 12), (250_000_00, 0.085, 60), (77_777_77, 0.045, 72), (50_000_00, 0.0, 24)] {
            let s = schedule(amount, rate, months);
            assert_eq!(s.len(), months as usize);
            assert_eq!(s.iter().map(|i| i.principal).sum::<Cents>(), amount);
            assert_eq!(s.last().unwrap().remaining, 0);
            // every rate but the last is the annuity, the interest falling with the debt
            let monthly = economy::annuity(amount, rate, months);
            assert!(s[..s.len() - 1].iter().all(|i| i.payment == monthly));
            assert!((s.last().unwrap().payment - monthly).abs() <= months as Cents);
            assert!(s.windows(2).all(|w| w[1].interest <= w[0].interest));
        }
        // the schedule is what the months book
        let mut c = found(&Founding { name: "Bank".into(), difficulty: Difficulty::Realistic, ..Default::default() }, "Luc");
        let k = LoanContract { signed_by: "Luc".into(), ..draft_loan(&c, 120_000_00, 24, "bus", 0) };
        sign_loan(&mut c, &k, 0).unwrap();
        let cash = c.cash;
        for _ in 0..24 {
            pay_rates(&mut c);
        }
        assert!(c.loans.is_empty());
        assert_eq!(cash - c.cash, k.total);
        assert_eq!(k.total, schedule(120_000_00, k.rate, 24).iter().map(|i| i.payment).sum::<Cents>());
    }

    #[test]
    fn the_credit_room_is_the_base_the_fleet_and_the_income_less_the_debt() {
        let mut c = found(&Founding { name: "Bank".into(), difficulty: Difficulty::Realistic, date: "2024-05-06".into(), ..Default::default() }, "Luc");
        let cr = credit(&c, 0);
        assert_eq!((cr.base, cr.fleet_value, cr.income, cr.debt), (600_000_00, 0, 0, 0));
        assert_eq!((cr.limit, cr.left), (600_000_00, 600_000_00));
        assert_eq!(room(&cr), Room::Plenty);
        // what a loan buys counts for its share
        assert_eq!(credit(&c, 100_000_00).limit, 660_000_00);
        // three good months raise it; a bad one does not take it under nothing
        c.months.push(super::super::model::Month { month: "2024-02".into(), by_kind: vec![(BookingKind::Fares, 30_000_00)] });
        c.months.push(super::super::model::Month { month: "2024-04".into(), by_kind: vec![(BookingKind::Fares, 30_000_00)] });
        assert_eq!(monthly_income(&c), 20_000_00);
        assert_eq!(credit(&c, 0).limit, 600_000_00 + 6 * 20_000_00);
        c.months.push(super::super::model::Month { month: "2024-03".into(), by_kind: vec![(BookingKind::Wages, -200_000_00)] });
        assert_eq!(monthly_income(&c), 0);
        // the debt eats it: little left, then none
        take_loan(&mut c, 500_000_00, "x", 0).unwrap();
        assert_eq!(room(&credit(&c, 0)), Room::Little);
        take_loan(&mut c, 95_000_00, "y", 0).unwrap();
        let cr = credit(&c, 0);
        assert_eq!((cr.left, room(&cr)), (5_000_00, Room::None));
        assert!(take_loan(&mut c, 10_000_00, "z", 0).is_err());
        // a higher level lends cheaper
        assert_eq!(rate_of(&c), 0.045);
        c.progress.xp = levels::LEVEL_XP[9];
        assert!((rate_of(&c) - 0.035).abs() < 1e-9 && (credit(&c, 0).discount - 0.01).abs() < 1e-9);
        // the terms on offer, up to the difficulty's longest
        assert_eq!(terms_offered(&c), vec![12, 24, 36, 48, 60, 72]);
    }

    #[test]
    fn a_loan_is_taken_by_its_signed_contract_and_paid_back_early_with_a_fee() {
        let mut c = found(&Founding { name: "Bank".into(), difficulty: Difficulty::Realistic, date: "2024-05-06".into(), ..Default::default() }, "Luc");
        let k = draft_loan(&c, 100_000_00, 36, "New buses", 0);
        assert_eq!((k.first_rate.as_str(), k.months, k.lender.as_str(), k.borrower.as_str()), ("2024-05-31", 36, BANK, "Bank"));
        assert!(k.interest() > 0 && k.total == k.amount + k.interest());
        // unsigned: nothing
        assert_eq!(sign_loan(&mut c, &k, 0), Err("Sign the contract first."));
        assert!(c.loans.is_empty());
        let cash = c.cash;
        let k = LoanContract { strokes: vec![vec![[0.1, 0.1], [0.5, 0.5]]], ..k };
        let id = sign_loan(&mut c, &k, 0).unwrap();
        assert_eq!(c.cash, cash + 100_000_00);
        assert_eq!((c.loans[0].months_left, c.loans[0].monthly), (36, k.monthly));
        assert_eq!(c.dealer.loan_contracts[0].no, id);
        // early: 1 % on Realistic
        let cash = c.cash;
        repay(&mut c, id, 40_000_00).unwrap();
        assert_eq!(cash - c.cash, 40_000_00 + 400_00);
        assert_eq!(c.debt(), 60_000_00);
        // a loan and what it pays for, all or nothing
        let k2 = LoanContract { signed_by: "Luc".into(), ..draft_loan(&c, 50_000_00, 12, "bus", 50_000_00) };
        let before = c.clone();
        assert_eq!(with_loan(&mut c, &k2, 50_000_00, |_| -> Result<(), &'static str> { Err("Not enough cash.") }), Err("Not enough cash."));
        assert_eq!(c, before);
        let (id2, ()) = with_loan(&mut c, &k2, 50_000_00, |c| {
            c.book(BookingKind::Purchase, -50_000_00, "bus", false);
            Ok(())
        })
        .unwrap();
        assert!(c.loans.iter().any(|l| l.id == id2));
    }

}
