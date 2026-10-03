"""Tells whether an issue is written in English, for .github/workflows/issue_language.yml.

Only the prose counts: code blocks, logs, links, images, HTML and the issue form's own
headings are dropped first, so a crash log with a Cyrillic user folder or a form whose
answers are in Chinese is judged by what the reporter wrote, not by what surrounds it.

Run as a script it prints `english` or `not-english` for the issue in $GITHUB_EVENT_PATH.
"""
import json
import os
import re
import sys
import unicodedata

# The most frequent short words of English and of the languages issues arrive in. A text is
# English when its English function words outnumber every other language's.
STOPWORDS = {
    "en": "the a an and or but of to in on at for with from by is are was were be been it this that these those i you he she we they my your not no do does did have has had can could will would should when if then there what which who how after before while so as also just only still all any some".split(),
    "de": "der die das und oder aber nicht ist sind war waren ein eine einen dem den des mit von zu bei auf für ich du er sie wir ihr es auch noch wenn dann nach vor wie was wird werden kann bitte seit hat haben kein keine keinen sich man nur im am zum zur beim".split(),
    "pt": "o os as um uma uns umas e ou mas não é são era foi de do da dos das em no na nos nas com por para que se eu você ele ela nós eles mais quando depois antes também ainda".split(),
    "es": "el la los las un una y o pero no es son era fue de del en con por para que se yo tú él ella nosotros ellos más cuando después antes también todavía muy".split(),
    "fr": "le la les un une et ou mais ne pas est sont était de du des en dans avec pour par que qui je tu il elle nous vous ils plus quand après avant aussi encore très".split(),
    "it": "il lo la gli le un una e o ma non è sono era di del della in con per che si io tu lui lei noi voi loro più quando dopo prima anche ancora molto".split(),
    "nl": "de het een en of maar niet is zijn was waren van in op met voor door dat die ik jij hij zij wij ook nog wanneer als na".split(),
    "pl": "i w z na nie jest są był była się do że to ten ta od po przy jak ale lub czy jeśli gdy też jeszcze bardzo".split(),
    "cs": "a v z na ne je jsou byl byla se do že to ten ta od po při jak ale nebo pokud když také ještě velmi".split(),
    "tr": "ve veya ama değil bir bu şu o ile için gibi da de ne zaman sonra önce çok daha".split(),
    "hu": "a az és vagy de nem van volt egy ez az hogy mert ha amikor után előtt is még nagyon".split(),
}

# What the issue forms write themselves.
FORM_NOISE = re.compile(r"^(#{1,6} .*|_No response_|- \[[ xX]\] .*)$", re.M)


def prose(text: str) -> str:
    text = re.sub(r"```.*?(```|$)", " ", text, flags=re.S)  # fenced code and logs
    text = re.sub(r"`[^`\n]*`", " ", text)  # inline code
    text = re.sub(r"<!--.*?-->", " ", text, flags=re.S)
    text = re.sub(r"<[^>]+>", " ", text)  # HTML, <img> tags
    text = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", text)  # keep a link's words, drop its address
    text = re.sub(r"https?://\S+|www\.\S+", " ", text)
    text = re.sub(r"\S+[\\/]\S+", " ", text)  # file paths
    return FORM_NOISE.sub(" ", text)


def is_latin(ch: str) -> bool:
    return "LATIN" in unicodedata.name(ch, "")


def verdict(title: str, body: str) -> tuple[bool, str]:
    text = prose(title + "\n" + (body or ""))
    letters = [c for c in text if c.isalpha()]
    other = sum(1 for c in letters if not is_latin(c))
    # Another script (Cyrillic, Chinese, Korean...): English uses none, so a fair share of
    # such letters settles it, whatever the Latin words around them say.
    if other >= 8 and other > 0.2 * len(letters):
        return False, f"{other} of {len(letters)} letters are not Latin"
    words = re.findall(r"[^\W\d_]+", text.lower())
    if len(words) < 6:
        return True, "too short to tell"
    hits = {lang: sum(1 for w in words if w in set(sw)) for lang, sw in STOPWORDS.items()}
    best = max((lang for lang in hits if lang != "en"), key=hits.get)
    # Several languages share words (a, de, in, is...), so another language must clearly
    # lead before an issue counts as not English.
    if hits[best] >= 4 and hits[best] > 1.5 * hits["en"]:
        return False, f"{best} words {hits[best]}, English words {hits['en']}"
    # A short report without a single English function word, but with some of another
    # language's: "Kopfsteinpflaster hat keinen Effekt".
    if hits["en"] == 0 and hits[best] >= 2:
        return False, f"{best} words {hits[best]}, no English words"
    return True, f"English words {hits['en']}, {best} words {hits[best]}"


if __name__ == "__main__":
    issue = json.load(open(os.environ["GITHUB_EVENT_PATH"], encoding="utf-8"))["issue"]
    english, why = verdict(issue["title"], issue.get("body") or "")
    print(why, file=sys.stderr)
    print("english" if english else "not-english")
