//! `OMSI_RECORD=<fps>`: an offscreen `--drive` run written as a film with its sound - a
//! picture every 1/fps second into `<out>_frames/` (JPEG) and everything the game would
//! have played, mixed for the same simulated time, into `<out>.wav` - however long a
//! picture takes to draw. The audio engine is a capture engine (`AudioEngine::capture`):
//! the bus's own sound configuration, the traffic's, OMSI's rain in the street, mixed
//! step by step. `ffmpeg -framerate <fps> -i <out>_frames/f%05d.jpg -i
//! <out>.wav -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest film.mp4` makes the film.
//! `OMSI_RECORD_FROM=<s>` starts the film that far into the drive (the bus started, the
//! traffic come).

use super::*;
use crate::view_sync::{self, ViewSync};

/// The film's sample rate.
const RATE: u32 = 48_000;

pub(super) struct Recorder {
    audio: omsi_audio::AudioEngine,
    ambience: crate::ambience::Ambience,
    fps: f32,
    from: f32,
    dir: PathBuf,
    wav: PathBuf,
    /// The mixed sound so far (16-bit stereo) and how many frames of it were mixed.
    samples: Vec<i16>,
    mixed: u64,
    pictures: usize,
    peak: f32,
}

impl Recorder {
    /// A recorder for the run writing to `out`, when `OMSI_RECORD` asks for one.
    pub(super) fn new(out: &Path, player: Option<&mut Player>, root: &Path) -> Option<Recorder> {
        let fps = omsi_cfg::flags::OMSI_RECORD.parse::<f32>().filter(|f| *f > 0.0 && *f <= 60.0)?;
        let from = omsi_cfg::flags::OMSI_RECORD_FROM.parse::<f32>().unwrap_or(0.0).max(0.0);
        let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("film").to_string();
        let dir = out.with_file_name(format!("{stem}_frames"));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("record: {}: {e}", dir.display());
            return None;
        }
        let audio = omsi_audio::AudioEngine::capture(RATE);
        if let Some(p) = player {
            p.load_sounds(&audio);
        }
        let ambience = crate::ambience::Ambience::load(&audio, root);
        log::info!("record: {fps} pictures a second from {from} s into {}, the sound into {stem}.wav", dir.display());
        Some(Recorder { audio, ambience, fps, from, dir, wav: out.with_file_name(format!("{stem}.wav")), samples: Vec::new(), mixed: 0, pictures: 0, peak: 0.0 })
    }
}

impl Offscreen<'_> {
    /// The camera of this moment: the player's view (the head turned as --look says), a
    /// followed car's, the run's own.
    fn camera_now(&mut self) -> Camera {
        let args = self.args;
        let mut cam = self.camera;
        if let Some(p) = self.player.as_mut() {
            pose_player(p, &self.renderer, &mut self.scene, args, &self.settings);
            if args.cam.is_none() && args.view != "free" && args.follow.is_none() {
                cam = player_view(args, &self.settings, p, &self.camera, &self.world);
            }
            vehicle_camera(p, &mut cam);
        }
        if let Some(id) = follow_id(args, self.traffic.as_ref()) {
            if let Some(c) = follow_camera(self.traffic.as_ref(), id) {
                cam = c;
            }
        }
        cam
    }

    /// The lighting of this moment seen from `cam`.
    pub(super) fn lighting_now(&mut self, cam: &Camera) -> omsi_render::Lighting {
        let daylight = omsi_sim::Daylight::compute(&self.run_clock, self.envir.as_ref());
        let driven = self.player.as_ref().map(|p| &p.vehicle);
        let _ = cam;
        steps::picture_lighting(
            &daylight,
            Some(&self.weather),
            cloud_drift_at(&self.weather, self.run_clock.time),
            self.wetness,
            Some(&self.world),
            driven,
            driven,
            self.cabin_air.appearance(),
            &self.settings,
            self.run_clock.run_time as f32,
        )
    }

    /// A picture of this moment (as `--snapshots` takes it) and its camera.
    pub(super) fn picture_now(&mut self) -> Result<(Vec<u8>, Camera)> {
        if let Some(t) = self.traffic.as_mut() {
            view_sync::sync(ViewSync::traffic(t), &mut self.sim_view, &self.world, &self.renderer, &mut self.scene);
        }
        let cam = self.camera_now();
        if let Some(h) = self.humans_off.as_mut() {
            view_sync::sync(ViewSync::people(h, None, cam.position), &mut self.sim_view, &self.world, &self.renderer, &mut self.scene);
        }
        // the time of day of this moment, and its lights: street lamps by night and the
        // vehicles' own (indicators, brake and tail lights) as they are now - without them
        // a snapshot showed no vehicle light at all
        let daylight = omsi_sim::Daylight::compute(&self.run_clock, self.envir.as_ref());
        steps::world_lamps(&self.world, &self.renderer, &mut self.scene, &self.run_clock, &daylight, true, true);
        {
            let vehicles = steps::light_vehicles(self.player.as_ref(), self.traffic.as_ref(), &self.remotes_off);
            world_lights(&mut self.renderer, &self.world, &mut self.scene, &self.weather, &daylight, cam.position, &vehicles);
        }
        self.spray.sprites(cam.position, &mut self.scene.smoke);
        let lighting = self.lighting_now(&cam);
        self.world.finish_texture_upgrades(&self.renderer, &mut self.scene);
        let pixels = self.renderer.render_to_image(&mut self.scene, self.w, self.h, &cam, &lighting)?;
        Ok((pixels, cam))
    }

    /// The recording's step `i` (`t_s` into the run): this step's sound, and the picture
    /// when one is due.
    pub(super) fn record_step(&mut self, i: usize, t_s: f32) -> Result<()> {
        let Some(mut rec) = self.recorder.take() else { return Ok(()) };
        let result = self.record_with(&mut rec, i, t_s);
        self.recorder = Some(rec);
        result
    }

    fn record_with(&mut self, rec: &mut Recorder, i: usize, t_s: f32) -> Result<()> {
        let dt = self.dt;
        let cam = self.camera_now();
        let inside = matches!(self.args.view.as_str(), "driver" | "pax");
        let a = &rec.audio;
        let (reverb_time, reverb_mix) = self.world.reverb_at(cam.position);
        a.set_listener(omsi_audio::Listener {
            position: cam.position.as_vec3(),
            forward: cam.forward(),
            right: cam.right(),
            master: self.settings.volume.clamp(0.0, 1.0),
            reverb_time,
            reverb_mix,
        });
        if let Some(p) = self.player.as_mut() {
            p.sound_step(Some(a), inside, inside);
        }
        let street = crate::weather_setup::street_condition(&self.weather, self.wetness);
        if let Some(t) = self.traffic.as_mut() {
            t.update_audio(a, cam.position, street, inside, None);
        }
        rec.ambience.update(a, dt, precip_of(&self.weather), inside, street, cam.position, &[]);
        // the sound of this step, to the sample (no drift between the picture and the sound)
        let due = (((i + 1) as f64) * dt as f64 * RATE as f64).round() as u64;
        let n = due.saturating_sub(rec.mixed) as usize;
        let mixed = a.render_capture(n);
        rec.mixed = due;
        let filming = t_s + 1.0e-4 >= rec.from;
        if filming {
            for x in &mixed {
                rec.peak = rec.peak.max(x.abs());
                rec.samples.push((x * 32767.0).round().clamp(-32768.0, 32767.0) as i16);
            }
        }
        if i.is_multiple_of(30) {
            log::info!("record: {:.1} s", t_s);
        }
        // the pictures: one each 1/fps second of film
        let film_t = t_s - rec.from;
        let want = if filming { ((film_t + dt) * rec.fps).floor() as usize } else { 0 };
        while filming && rec.pictures < want {
            let (pixels, _) = self.picture_now()?;
            let path = rec.dir.join(format!("f{:05}.jpg", rec.pictures));
            let rgb: Vec<u8> = pixels.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            let file = std::fs::File::create(&path)?;
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), 92);
            enc.encode(&rgb, self.w, self.h, image::ExtendedColorType::Rgb8)?;
            rec.pictures += 1;
        }
        Ok(())
    }

    /// The recording's end: the sound into its WAV file.
    pub(super) fn finish_recording(&mut self) -> Result<()> {
        let Some(rec) = self.recorder.take() else { return Ok(()) };
        write_wav(&rec.wav, RATE, 2, &rec.samples)?;
        log::info!(
            "record: {} pictures in {}, {:.1} s of sound in {} (peak {:.1} dBFS)",
            rec.pictures,
            rec.dir.display(),
            rec.samples.len() as f32 / 2.0 / RATE as f32,
            rec.wav.display(),
            20.0 * rec.peak.max(1.0e-9).log10()
        );
        Ok(())
    }
}

/// A 16-bit PCM WAV file.
fn write_wav(path: &Path, rate: u32, channels: u16, samples: &[i16]) -> Result<()> {
    use std::io::Write;
    let data = samples.len() as u32 * 2;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&channels.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&(rate * channels as u32 * 2).to_le_bytes())?;
    f.write_all(&(channels * 2).to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data.to_le_bytes())?;
    for s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_wav_written_reads_back() {
        let dir = std::env::temp_dir().join(format!("omsi_rec_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.wav");
        let samples: Vec<i16> = (0..960).map(|k| (k as i16 - 480) * 60).collect();
        super::write_wav(&path, 48_000, 2, &samples).unwrap();
        let back = omsi_audio::wav::parse_wav(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!((back.sample_rate, back.channels), (48_000, 2));
        assert_eq!(back.samples, samples);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
