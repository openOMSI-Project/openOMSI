//! People coming and going: making somebody (`spawn`), taking them away, the pool they
//! are counted against, and the setting up of `PeopleSim`.

use super::*;

impl PeopleSim {
    /// LAN uses the room id as the shared source of randomness.  This keeps the
    /// initial pedestrian selection and their generated identities identical on
    /// the host and clients; subsequent movement remains simulation-local.
    pub fn set_lan_seed(&mut self, seed: u64) {
        self.rng = (seed ^ 0xA5A5_5A5A_1F2E_3D4C) as u64 | 1;
    }

    /// `configured_people`: the `ai_max_humans` setting (see `max_people`).
    pub fn new(root: &Path, configured_people: usize) -> PeopleSim {
        let mut types = Vec::new();
        // `Humans/<group>/*.hum` of every content root (an installed map or mod brings its
        // own people); a file of the same group and name higher up replaces the stock one
        let mut roots = omsi_cfg::content_dirs("Humans");
        if roots.is_empty() {
            roots.push(root.join("Humans"));
        }
        // (group, file name, path), sorted by group and name as the single folder used to be
        let mut found: Vec<(std::ffi::OsString, std::ffi::OsString, std::path::PathBuf)> =
            Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for r in &roots {
            for (group, is_dir) in omsi_cfg::vfs::list_dir(r).unwrap_or_default() {
                if !is_dir {
                    continue;
                }
                let d = r.join(&group);
                for (n, _) in omsi_cfg::vfs::list_dir(&d).unwrap_or_default() {
                    let lower = n.to_string_lossy().to_ascii_lowercase();
                    if !lower.ends_with(".hum") || lower.contains("driver") {
                        continue;
                    }
                    if seen.insert(format!(
                        "{}/{lower}",
                        group.to_string_lossy().to_ascii_lowercase()
                    )) {
                        found.push((group.clone(), n.clone(), d.join(&n)));
                    }
                }
            }
        }
        found.sort();
        let files: Vec<std::path::PathBuf> = found.into_iter().map(|(_, _, p)| p).collect();
        for f in files {
            match HumanType::load(&f) {
                Ok(t) => types.push(Arc::new(t)),
                Err(e) => log::warn!("human {}: {e:#}", f.display()),
            }
        }
        log::info!("humans: {} types", types.len());
        if omsi_cfg::flags::OMSI_DEBUG_HUMANS.is_set() {
            for t in &types {
                let (mut lo, mut hi) = (f32::MAX, f32::MIN);
                for m in &t.meshes {
                    for v in &m.data.positions {
                        lo = lo.min(v.z);
                        hi = hi.max(v.z);
                    }
                }
                log::info!(
                    "  {} z {lo:.2}..{hi:.2}",
                    t.def.path.file_name().unwrap_or_default().to_string_lossy()
                );
            }
        }
        let people_limit = bounded_people_limit(configured_people);
        if people_limit != configured_people {
            log::warn!("human pool limit {configured_people} adjusted to {people_limit} for safe spawning");
        }
        PeopleSim {
            types,
            people: Vec::new(),
            rng: 0x1234_5678_9ABC_DEF1,
            next_id: 1,
            time: 0.0,
            wall_cells: HashMap::new(),
            wall_key: (0, 0, 0, 0.0),
            cabins: HashMap::new(),
            player_cabin: None,
            player_next_stop: None,
            seats: HashMap::new(),
            stops: HashMap::new(),
            odometer: HashMap::new(),
            pax_req: HashMap::new(),
            pax_busy: HashMap::new(),
            pax_places: HashMap::new(),
            desk_busy: None,
            pardons: 0,
            pardon_max: 0,
            started: false,
            ped: None,
            bodies: BodyOps::default(),
            served_stop: None,
            ai_visits: HashMap::new(),
            last_door_open: HashMap::new(),
            runner_rolls: HashMap::new(),
            holds: Vec::new(),
            ai_requests: Vec::new(),
            tickets: None,
            request: None,
            paid: None,
            change_due: None,
            money: None,
            under_bus: hashbrown::HashSet::new(),
            stop_request: false,
            tickets_sold: 0,
            ticket_cash: 0.0,
            sales: Vec::new(),
            boarded: 0,
            served: 0,
            stepped_in: 0,
            content: 0,
            ticket_requests: 0,
            ticket_points: 0,
            entry_req: Vec::new(),
            exit_req: Vec::new(),
            entry_busy: Vec::new(),
            exit_busy: Vec::new(),
            footfalls: Vec::new(),
            density: 1.0,
            time_of_day: 12.0 * 3600.0,
            delay: 0.0,
            root: root.to_path_buf(),
            voice_lines: Vec::new(),
            voice_said: HashMap::new(),
            voices: 0,
            last_chat: -1e9,
            avatars: HashMap::new(),
            avatar_cmds: HashMap::new(),
            avatar_hidden: HashMap::new(),
            last_buses: Vec::new(),
            avatar_only: false,
            driver_away: false,
            stop_targets: None,
            due_dests: None,
            due_at: f64::NEG_INFINITY,
            stop_names: None,
            duty: None,
            stamped: Vec::new(),
            pedestrians: 14,
            max_people: people_limit,
            stroll_timer: 0.0,
            exact_fare: true,
            boarding: "auto".into(),
            give_ticket: false,
            give_change_all: false,
            ticket_key: "T".into(),
            eye: None,
            center: DVec3::ZERO,
            message: None,
            tick_stats: (0, 0.0, 0.0),
            tick_stages: Vec::new(),
            bus_motion: HashMap::new(),
            map_humans_done: false,
            tiles_seen: 0,
            mirror: false,
            lan_centers: Vec::new(),
            players_only: false,
            remote_now: Vec::new(),
            placed_now: Vec::new(),
            claims_out: Vec::new(),
            claimed: HashMap::new(),
            mirror_wait: HashMap::new(),
            comfort: RideComfort::default(),
            handed: Vec::new(),
        }
    }

    /// The map's tiles changed: a stop gone with its tile takes the people waiting there
    /// with it; a stop nobody waits at is set up again with what its tiles hold now (its
    /// waiting places come with the objects round it, sub_620c0c).
    pub fn tiles_changed(&mut self, world: &dyn World) {
        let present: HashSet<i64> = world.bus_stops().iter().map(|s| s.0).collect();
        let bound = |st: &State| -> Option<i64> {
            match st {
                State::Pax(p) if p.inside.is_none() => p.stop,
                _ => None,
            }
        };
        let used: HashSet<i64> = self.people.iter().filter_map(|p| bound(&p.state)).collect();
        let gone: Vec<i64> = self.stops.keys().copied().filter(|id| !present.contains(id)).collect();
        let mut removed = 0usize;
        for i in (0..self.people.len()).rev() {
            let p = &self.people[i];
            let lost_stop = bound(&p.state).is_some_and(|s| gone.contains(&s));
            let lost_ground = p.place == Place::Ground && p.puppet.is_none() && !world.has_ground(p.position.x, p.position.y);
            if lost_stop || lost_ground {
                self.release(i);
                let p = self.people.swap_remove(i);
                if debug_pax() {
                    log::info!("t={:.1} pax {} taken away with its tile ({})", self.time, p.label(), p.state.name());
                }
                self.retire(&p);
                removed += 1;
            }
        }
        for id in &gone {
            self.stops.remove(id);
        }
        let idle: Vec<i64> = self.stops.keys().copied().filter(|id| !used.contains(id)).collect();
        let rebuilt = idle.len();
        for id in idle {
            self.stops.remove(&id);
        }
        if debug_pax() || (omsi_cfg::flags::OMSI_PROFILE.is_set() && (removed > 0 || !gone.is_empty())) {
            log::info!("people: tiles changed: {} stops gone, {rebuilt} set up again, {removed} people taken away", gone.len());
        }
    }

    /// `limited`: said only when the same file has not been said for 10 s (greetings and
    /// complaints; the ticket asked for, "thanks" and the missing change always are).
    pub fn say_ex(&mut self, i: usize, name: &str, limited: bool) {
        // the player may have silenced them (settings), all but the ticket they ask for
        match self.voices {
            2 => return,
            1 if !name.starts_with("Ticket_") => return,
            _ => {}
        }
        // Greetings and complaints: one at a time for the whole bus. OMSI only keeps
        // the same file from being said twice within 10 s, and with a dozen people
        // boarding every other one said hello - the saloon never stopped talking, which
        // is not how the original sounds: a few words now and then.
        if limited && self.time - self.last_chat < CHAT_PAUSE && self.time >= self.last_chat {
            return;
        }
        // (without a `[voicepath]` the pack's own folder: Berlin_1 and Berlin_86 carry the
        // voices themselves and name no path; the later packs point at theirs)
        let Some(base) = self.tickets.as_ref().and_then(|t| match &t.voice_path {
            Some(vp) if !vp.trim().is_empty() => Some(omsi_cfg::resolve_path(&self.root, vp.trim())),
            _ => t.path.parent().map(|p| p.to_path_buf()),
        }) else {
            return;
        };
        let voice = self.people[i].ty.def.voice.trim().to_string();
        if voice.is_empty() {
            return;
        }
        let dir = omsi_cfg::resolve_path(&base, &voice);
        let path = omsi_cfg::resolve_path(&dir, &format!("{name}.wav"));
        if !omsi_cfg::vfs::is_file(&path) {
            return;
        }
        if limited {
            if let Some(&t) = self.voice_said.get(&path) {
                if self.time - t < 10.0 && self.time >= t {
                    return;
                }
            }
        }
        self.voice_said.insert(path.clone(), self.time);
        if limited {
            self.last_chat = self.time;
        }
        if debug_pax() {
            log::info!("t={:.1} pax {} says {name}", self.time, self.people[i].label());
        }
        self.voice_lines.push(VoiceLine { position: self.people[i].position + DVec3::new(0.0, 0.0, 1.6), path });
    }

    pub fn rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn rand_f(&mut self) -> f64 {
        (self.rand() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Whether the player could see somebody standing at `p`.
    pub fn seen(&self, p: DVec3) -> bool {
        match self.eye {
            None => (p - self.center).length() < 150.0,
            Some(e) => {
                let d = p + DVec3::Z * 0.9 - e.pos;
                let dist = d.length();
                if dist > 230.0 {
                    return false;
                }
                dist < 3.0 || d.dot(e.fwd) / dist > e.cos_half
            }
        }
    }

    /// The people of OMSI's pool there are now (everybody but avatars and other players'
    /// people mirrored here).
    pub fn pool_used(&self) -> usize {
        self.people.iter().filter(|p| p.puppet.is_none() && !p.remote).count()
    }

    /// Room in the pool for one more person; when it is full somebody walking the street out
    /// of sight is taken for it, as Omsi.exe takes a task-8 person for a stop (0x61bd44).
    pub fn pool_room(&mut self) -> bool {
        if self.pool_used() < bounded_people_limit(self.max_people) {
            return true;
        }
        let free = (0..self.people.len()).find(|&i| {
            let p = &self.people[i];
            // (not somebody put out to run for a bus, see `runners`)
            p.puppet.is_none() && !p.remote && matches!(p.state, State::Strolling(_) | State::Standing) && !self.seen(p.position) && !self.put_out(p.id)
        });
        match free {
            Some(i) => {
                self.release(i);
                let p = self.people.swap_remove(i);
                self.retire(&p);
                self.pool_used() < bounded_people_limit(self.max_people)
            }
            None => false,
        }
    }

    /// The cabin of a vehicle with the parts coupled behind it.
    pub fn cabin_for(&mut self, v: &VehicleInstance) -> Option<Arc<Cabin>> {
        let parts = train_parts(v);
        let key: Vec<PathBuf> = parts.iter().map(|p| p.0.path.clone()).collect();
        if let Some(c) = self.cabins.get(&key) {
            return c.clone();
        }
        let cabin = Cabin::load_train(&parts).map(Arc::new);
        if let Some(c) = cabin.as_ref().filter(|c| c.parts.len() > 1) {
            log::info!("passenger cabin of {}: {} sections joined ({} places, {} entries, {} exits, {} path points)", v.ty.def.path.file_name().unwrap_or_default().to_string_lossy(), c.parts.len(), c.seats.len(), c.entries.len(), c.exits.len(), c.graph.points.len());
        }
        self.cabins.insert(key, cabin.clone());
        cabin
    }

    /// Seats of the player's bus from its `[passengercabin]`, and the engine's side of the
    /// ticket printer: `GivenTicket` is -1 until the driver hands a ticket over (the stock
    /// `Ticketprinter.osc` never sets it, OMSI starts it at -1 - left at 0 the first
    /// passenger took ticket 0 without the driver doing anything).
    pub fn set_cabin(&mut self, vehicle: &mut VehicleInstance) {
        vehicle.set_engine_var("GivenTicket", -1.0);
        match self.cabin_for(vehicle) {
            Some(c) => {
                log::info!("passenger cabin: {} places ({} seats), {} entries, {} exits, {} path points, desk {:?}", c.seats.len(), c.seats.iter().filter(|s| s.seated).count(), c.entries.len(), c.exits.len(), c.graph.points.len(), c.desk.map(|d| d.0));
                if debug_pax() {
                    for (i, e) in c.entries.iter().enumerate() {
                        log::info!(
                            "  entry {i}: inside {:?} wait {:?} sells {}",
                            e.inside,
                            e.wait,
                            e.sells
                        );
                    }
                    for (i, e) in c.exits.iter().enumerate() {
                        log::info!("  exit {i}: inside {:?} wait {:?}", e.inside, e.wait);
                    }
                    for (i, s) in c.seats.iter().enumerate() {
                        log::info!(
                            "  seat {i}: pos {:?} floor {:?} rot {:.0} seated {}{}{}",
                            s.pos,
                            s.floor,
                            s.rot,
                            s.seated,
                            s.switch_var.as_ref().map(|n| format!(" switched by {n} ({:?})", vehicle.var(n))).unwrap_or_default(),
                            s.taken_var.as_ref().map(|n| format!(" occupancy into {n}")).unwrap_or_default()
                        );
                    }
                }
                self.seats.insert(BusId::Player, vec![false; c.seats.len()]);
                self.entry_req = vec![false; c.entries.len().max(1)];
                self.exit_req = vec![false; c.exits.len().max(1)];
                self.entry_busy = vec![false; c.entries.len()];
                self.exit_busy = vec![false; c.exits.len()];
                self.player_cabin = Some(c);
            }
            None => log::info!("{}: no passenger cabin", vehicle.ty.def.path.display()),
        }
    }

    pub fn spawn(
        &mut self,
        world: &dyn World,
        position: DVec3,
        heading: f64,
        state: State,
    ) -> Option<usize> {
        // All local creation paths share the pool, including test riders and crossing
        // pedestrians. Avatars and host mirrors use spawn_as directly and keep their
        // existing ownership; never evict somebody aboard a bus to make room.
        if !self.pool_room() {
            return None;
        }
        self.spawn_as(world, position, heading, state, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_as(
        &mut self,
        world: &dyn World,
        position: DVec3,
        heading: f64,
        state: State,
        kind: Option<usize>,
    ) -> Option<usize> {
        self.use_map_humans(world);
        if self.types.is_empty() {
            return None;
        }
        // on the surface they will walk on, not on the bare terrain under a pavement
        // (they stood in the asphalt and climbed out of it when they started walking)
        let mut position = position;
        if let Some(z) = world.walk_height_near(position.x, position.y, position.z) {
            if (z - position.z).abs() < 3.0 {
                position.z = z;
            }
        }
        // Not a twin of somebody standing near: two of the same figure in the same clothes
        // side by side at a stop was the first thing one noticed. A few tries for a figure
        // nobody near wears (then at least other clothes); with few figures installed some
        // repeat anyway.
        let near: Vec<(usize, usize)> = self
            .people
            .iter()
            .filter(|q| (q.position - position).truncate().length() < 30.0)
            .map(|q| (Arc::as_ptr(&q.ty) as usize, q.variant))
            .collect();
        let mut choice: Option<(usize, usize)> = None;
        for attempt in 0..10 {
            let pick = (self.rand() % self.types.len() as u64) as usize;
            let idx = kind.map(|k| k % self.types.len()).unwrap_or(pick);
            let t = &self.types[idx];
            let tk = Arc::as_ptr(t) as usize;
            // the default clothes or one of the `.cti` variants, alike likely
            let n_var = t.variants.len() as u64 + 1;
            let v0 = (self.rand() % n_var) as usize;
            // a clothing variant nobody near wears in this figure
            let var = (0..n_var as usize).map(|k| (v0 + k) % n_var as usize).find(|v| !near.contains(&(tk, *v)));
            let figure_free = !near.iter().any(|n| n.0 == tk);
            match var {
                Some(v) if figure_free || attempt >= 6 || kind.is_some() => {
                    choice = Some((idx, v));
                    break;
                }
                Some(v) if choice.is_none() => choice = Some((idx, v)),
                None if choice.is_none() && attempt == 9 => choice = Some((idx, v0)),
                _ => {}
            }
        }
        let (idx, variant) = choice.unwrap_or((0, 0));
        let ty = self.types[idx].clone();
        // walking pace 1.1 m/s +- 0.2, as Omsi.exe draws it for everybody (0x625758:
        // sub_7f08b0(0.2, 1.1)); `[walk_param]` holds the stride, not a speed
        let pace = 1.1 + (self.rand_f() * 2.0 - 1.0) * 0.2;
        let age = ty.def.age.map(|a| a as f32).unwrap_or(40.0);
        let id = self.next_id;
        self.next_id += 1;
        // (the meshes and instances come when the view catches up: `show_bodies`)
        self.bodies.ops.push(BodyOp::Spawn { id, ty: ty.clone(), variant, position });
        if debug_pax() {
            log::info!(
                "pax #{id} ({}) appears at ({:.1}, {:.1}, {:.1}): {}{}",
                ty.def
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
                position.x,
                position.y,
                position.z,
                state.name(),
                if self.seen(position) { " IN SIGHT" } else { "" }
            );
        }
        self.people.push(Person {
            id,
            ty,
            variant,
            meshes: Vec::new(),
            position,
            heading,
            lheading: 0.0,
            place: Place::Ground,
            vel: DVec2::ZERO,
            pace,
            activity: Activity::Stand,
            anim: OmsiAnim::default(),
            state,
            t_state: 0.0,
            skins: Vec::new(),
            skin_bones: None,
            pose_changed: false,
            interior: 0.0,
            lit: 0.0,
            tilt: Mat4::IDENTITY,

            age,
            stuck: 0.0,
            ghost: 0.0,
            car_wait: 0.0,
            detour: 0.0,
            detour_side: 0.0,
            why: "",
            skinned: false,
            since_posed: 0,
            posed_at: (position, heading),
            ankles: [Vec3::ZERO; 2],
            puppet: None,
            remote: false,
        });
        Some(self.people.len() - 1)
    }

    /// A ticket of the pack for a passenger of `age`: those whose age
    /// range holds it, weighted by their probability - a day ticket's by the time of day
    /// as well (`day_ticket_factor`). `max_stations` plays no part in the choice.
    pub fn pick_ticket(&mut self, age: f32) -> Option<usize> {
        let r = self.rand_f() as f32;
        let day = day_ticket_factor(self.time_of_day);
        let t = self.tickets.as_ref()?;
        let weight = |tk: &omsi_content::tickets::Ticket| {
            if (tk.age_min as f32) > age || (tk.age_max as f32) < age {
                0.0
            } else if tk.day_ticket {
                tk.probability.max(0.0) * day
            } else {
                tk.probability.max(0.0)
            }
        };
        let total: f32 = t.tickets.iter().map(weight).sum();
        if total <= 0.0 {
            return None;
        }
        let mut x = r * total;
        for (i, tk) in t.tickets.iter().enumerate() {
            let w = weight(tk);
            if w > 0.0 && x < w {
                return Some(i);
            }
            x -= w;
        }
        None
    }

    pub fn free_seat(&mut self, bus: BusId, seat: usize) {
        if let Some(t) = self.seats.get_mut(&bus).and_then(|v| v.get_mut(seat)) {
            *t = false;
        }
    }

    /// Keep only the people the map's `humans.txt` names, an entry listed twice counting
    /// twice, as OMSI draws a map's pedestrians and passengers from that list alone. A map
    /// without the file, or whose list names nobody to be found, keeps everybody.
    pub fn use_map_humans(&mut self, world: &dyn World) {
        if self.map_humans_done {
            return;
        }
        self.map_humans_done = true;
        let path = omsi_cfg::resolve_path(world.map_dir(), "humans.txt");
        let list = omsi_map::ailists::load_list(&path);
        if list.is_empty() {
            return;
        }
        // the people installed already (any content root, mods too), matched by the path
        // below `Humans/`; an entry not among them (a pack nested deeper than the scan) is
        // loaded from its own path
        let key = |p: &str| -> String {
            let p = p.replace('\\', "/").to_ascii_lowercase();
            match p.rfind("humans/") {
                Some(k) => p[k + 7..].to_string(),
                None => p,
            }
        };
        let mut picked: Vec<Arc<HumanType>> = Vec::new();
        for line in &list {
            let want = key(line.trim());
            match self.types.iter().find(|t| key(&t.def.path.to_string_lossy()) == want) {
                Some(t) => picked.push(t.clone()),
                None => picked.extend(map_human_types(world.root(), std::slice::from_ref(line))),
            }
        }
        // (a list that names nobody to be found keeps everybody: a map without people
        // looked broken)
        if picked.is_empty() {
            log::warn!("humans.txt of the map names nobody installed: keeping all people");
            return;
        }
        log::info!(
            "humans: {} of {} map entries loaded from {}",
            picked.len(),
            list.len(),
            path.display()
        );
        self.types = picked;
    }

    /// People the moving bus has just knocked down. OMSI counts them in the driver's
    /// personnel file; they are only counted once and then walk away.
    pub fn run_over(&mut self, bus: &VehicleInstance) -> u32 {
        let moving = bus.physics.velocity_kmh().abs() >= 5.0;
        let Some(bb) = bus.ty.def.bounding_box.filter(|_| moving) else {
            self.under_bus.clear();
            return 0;
        };
        // (the box: its size and its centre in the bus frame - the size less the centre's
        // coordinate had been taken for the half size, and anyone standing in that box was
        // knocked down again every frame: 17 at once in one place, #1805)
        let (half_x, half_y, half_z) = (bb[0] / 2.0, bb[1] / 2.0, bb[2] / 2.0);
        let centre = Vec3::new(bb[3], bb[4], bb[5]);
        let inv = bus.body_rotation().transpose();
        let mut inside: hashbrown::HashSet<u32> = hashbrown::HashSet::new();
        let mut knocked = Vec::new();
        for (i, p) in self.people.iter().enumerate() {
            // (sub_62a6a0 at 0x62dc6c: the people waiting at a stop and those on the pavements)
            let counts = match &p.state {
                State::Pax(x) => x.task == Task::WaitingForBus,
                State::Strolling(_) | State::Standing => true,
                State::Idle => false,
            };
            if p.place != Place::Ground || !counts {
                continue;
            }
            let local = inv.transform_vector3((p.position - bus.position).as_vec3()) - centre;
            if local.x.abs() < half_x + 0.2 && local.y.abs() < half_y + 0.2 && local.z.abs() < half_z + 1.0 {
                inside.insert(p.id);
                if !self.under_bus.contains(&p.id) {
                    knocked.push(i);
                }
            }
        }
        self.under_bus = inside;
        let mut gone = Vec::new();
        for &i in knocked.iter().rev() {
            // a waiting passenger knocked down leaves the stop and walks off (sub_626818)
            if let State::Pax(x) = &self.people[i].state {
                let (at, h, stop) = (x.pos, x.yaw.to_degrees(), x.stop);
                self.release(i);
                let world = None::<&dyn World>;
                let _ = world;
                self.walk_street_plain(i, at, h, stop, &mut gone);
            }
        }
        gone.sort_unstable();
        for i in gone.into_iter().rev() {
            let p = self.people.swap_remove(i);
            self.retire(&p);
        }
        knocked.len() as u32
    }

    /// `--riders n`: n passengers already in their places in the player's bus (a test
    /// start; OMSI's buses start empty), without a destination - they ride 1..20 km.
    pub fn seed_riders(&mut self, n: usize, bus: &VehicleInstance, world: &dyn World) {
        let Some(cabin) = self.cabin_for(bus) else { return };
        let trailers = part_frames(bus, &cabin);
        let rot = bus.body_rotation();
        let off = places_off(bus, &cabin);
        for _ in 0..n {
            let Some(k) = self.reserve_place(BusId::Player, cabin.seats.len(), &off) else { break };
            let walk = 1.1 + (self.rand_f() as f32 * 2.0 - 1.0) * 0.2;
            let r = self.rand_f() as f32;
            let mut pax = Pax::new(walk, self.rand_f());
            pax.bus = Some(BusId::Player);
            pax.inside = Some(BusId::Player);
            pax.seat = Some(k);
            pax.ride_km = r * 19.0 + 1.0;
            pax.task = Task::InBusToPlace;
            let at = train_point(bus.position, &rot, &trailers, cabin.seats[k].pos);
            let Some(i) = self.spawn(world, at, bus.heading, State::Pax(Box::new(pax))) else {
                self.free_seat(BusId::Player, k);
                break;
            };
            let s = cabin.seats[k].clone();
            let seatheight = self.people[i].ty.def.seat_height;
            if let Some(p) = self.pax_mut(i) {
                p.task = Task::SittingInBus;
                p.st = 0;
                if s.seated {
                    p.seat_h = s.height;
                    p.pos = (s.pos - Vec3::Z * seatheight).as_dvec3();
                    p.pax_state = 2.0;
                } else {
                    p.pos = s.pos.as_dvec3();
                }
                p.yaw = (s.rot as f64).to_radians();
            }
            self.people[i].place = Place::Bus(BusId::Player, s.pos);
        }
    }

    /// Give back what a person holds (a waiting place, a seat) before they change plans.
    pub fn release(&mut self, i: usize) {
        let id = self.people[i].id;
        let State::Pax(x) = &mut self.people[i].state else { return };
        let (stop, spot, bus, seat) = (x.stop, x.spot.take(), x.bus.or(x.inside), x.seat.take());
        if let (Some(s), Some(k)) = (stop, spot) {
            self.free_spot(s, k);
        }
        if let (Some(b), Some(k)) = (bus, seat) {
            self.free_seat(b, k);
        }
        if self.desk_busy == Some(id) {
            self.desk_busy = None;
            self.request = None;
        }
    }
}
