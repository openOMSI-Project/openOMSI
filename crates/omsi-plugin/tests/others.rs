//! `omsi.others`: a plugin sees the vehicles around the bus and reads and writes their
//! variables (a trolleybus plugin putting the AI trolleybuses' poles on the wire).
use omsi_plugin::{HostConfig, Other, PluginIo, Plugins};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Default)]
struct Game {
    vars: HashMap<String, f32>,
    /// id -> (the vehicle, its variables)
    others: Vec<(Other, HashMap<String, f32>)>,
    messages: Vec<String>,
}

impl PluginIo for Game {
    fn system(&mut self, _: &str) -> Option<f32> {
        None
    }
    fn set_system(&mut self, _: &str, _: f32) {}
    fn has_vehicle(&self) -> bool {
        true
    }
    fn var(&mut self, name: &str) -> Option<f32> {
        self.vars.get(name).copied()
    }
    fn set_var(&mut self, name: &str, v: f32) {
        self.vars.insert(name.into(), v);
    }
    fn string(&mut self, _: &str) -> Option<String> {
        None
    }
    fn set_string(&mut self, _: &str, _: &str) {}
    fn fire(&mut self, _: &str, _: bool) {}
    fn position(&self) -> Option<[f64; 4]> {
        Some([0.0, 0.0, 0.0, 0.0])
    }
    fn message(&mut self, text: &str, _: f32) {
        self.messages.push(text.into());
    }
    fn others(&self, radius: f64) -> Vec<Other> {
        self.others.iter().map(|o| o.0.clone()).filter(|o| o.pos[0].hypot(o.pos[1]) <= radius).collect()
    }
    fn other_var(&mut self, id: u64, name: &str) -> Option<f32> {
        self.others.iter().find(|o| o.0.id == id)?.1.get(name).copied()
    }
    fn set_other_var(&mut self, id: u64, name: &str, v: f32) -> bool {
        match self.others.iter_mut().find(|o| o.0.id == id).and_then(|o| o.1.get_mut(name)) {
            Some(slot) => {
                *slot = v;
                true
            }
            None => false,
        }
    }
}

fn dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("omsi-lua-test-others-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("Poles")).unwrap();
    d
}

#[test]
fn others_are_listed_read_and_written() {
    let d = dir();
    std::fs::write(
        d.join("Poles/main.lua"),
        r#"
        function on_frame(dt)
          local list = omsi.others(100)
          omsi.set_var("count", #list)
          for _, o in ipairs(list) do
            if omsi.other_var(o.id, "pole") ~= nil then
              omsi.set_other_var(o.id, "pole", o.x)
              omsi.message(o.kind .. " " .. o.name)
            end
            assert(omsi.set_other_var(o.id, "no_such_var", 1) == false)
          end
        end
        "#,
    )
    .unwrap();
    let mut plugins = Plugins::load(&[d.clone()], &HostConfig::default());
    let mut g = Game::default();
    g.vars.insert("count".into(), 0.0);
    let trolley = Other { id: 7, kind: "ai", name: "ZiU 682G".into(), pos: [30.0, 40.0, 0.0, 90.0] };
    let car = Other { id: 8, kind: "player", name: "MAN SD202".into(), pos: [10.0, 0.0, 0.0, 0.0] };
    let far = Other { id: 9, kind: "ai", name: "far".into(), pos: [500.0, 0.0, 0.0, 0.0] };
    g.others.push((trolley, HashMap::from([("pole".to_string(), 0.0)])));
    g.others.push((car, HashMap::new()));
    g.others.push((far, HashMap::from([("pole".to_string(), 0.0)])));
    plugins.frame(&mut g);
    assert_eq!(g.vars["count"], 2.0, "the one 500 m off is out of the radius");
    assert_eq!(g.others[0].1["pole"], 30.0);
    assert_eq!(g.others[2].1["pole"], 0.0);
    assert_eq!(g.messages, ["ai ZiU 682G"]);
    plugins.finalize();
}
