use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use std::fs::File;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use console_engine::{Color, KeyCode};
use rodio::source::SineWave;
use rodio::{Sink, Source};
use serde::{Deserialize, Serialize};
use signal_hook::{consts::SIGUSR1, iterator::Signals};

#[derive(Debug, Deserialize, Serialize)]
struct Section {
    name: String,
    #[serde(skip_serializing, rename(deserialize = "time"))]
    pb_total: Option<u32>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        skip_deserializing,
        rename(serialize = "time")
    )]
    current_total: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
struct App {
    game: String,
    // name, pb and current time in sections
    sections: Vec<Section>,
    #[serde(skip)]
    current_section: usize,
    #[serde(skip, default = "Instant::now")]
    start_time: Instant,
    #[serde(skip, default = "chrono::Local::now")]
    start_date: chrono::DateTime<chrono::Local>,
    #[serde(skip)]
    running: bool,
}

impl App {
    fn handle_signal(app: &RwLock<Self>, sink: &Sink, sig: i32) -> Result<()> {
        if sig != SIGUSR1 {
            return Ok(());
        }

        let app = &mut app.write().expect("RwLock not poisoned");

        if app.current_section >= app.sections.len() {
            return Ok(());
        }

        if app.running == false {
            app.running = true;
            app.start_time = Instant::now();
            app.start_date = chrono::Local::now();
            app.current_section = 0;

            let source = SineWave::new(1.5 * 440.0)
                .take_duration(Duration::from_secs_f32(0.1))
                .amplify(0.20);
            sink.append(source.clone());

            return Ok(());
        }

        let source = SineWave::new(440.0)
            .take_duration(Duration::from_secs_f32(0.1))
            .amplify(0.20);
        sink.append(source.clone());

        let current_section = app.current_section;
        app.sections[current_section].current_total =
            Some(app.start_time.elapsed().as_millis() as u32);
        app.current_section += 1;

        if app.current_section >= app.sections.len() {
            // Run finished
            app.save()?;

            let source = SineWave::new(0.5 * 440.0)
                .take_duration(Duration::from_secs_f32(0.5))
                .amplify(0.20);
            sink.append(source.clone());

            return Ok(());
        }

        Ok(())
    }
    fn spawn_signal_handler(app: Arc<RwLock<Self>>) -> Result<()> {
        let mut signals = Signals::new(&[SIGUSR1])?;
        let (_stream, audio_stream_handle) = rodio::OutputStream::try_default()?;
        let sink = Sink::try_new(&audio_stream_handle)?;

        std::thread::spawn(move || {
            for sig in signals.forever() {
                Self::handle_signal(&app, &sink, sig)?;
            }

            Ok::<_, anyhow::Error>(())
        });

        Ok(())
    }

    fn launch_ui(app: &RwLock<Self>) -> Result<()> {
        let mut engine = console_engine::ConsoleEngine::init(50, 25, 10)?;
        loop {
            engine.wait_frame();
            engine.clear_screen();

            let app = &mut app.write().expect("RwLock not poisoned");
            app.update_current_time();

            engine.print(0, 0, &format!(" speedy: {}", app.game));
            engine.print(0, 1, " section | best  | current       | section      ");
            engine.print(0, 2, " --------|-------|---------------|--------------");
            for (i, s) in app.sections.iter().enumerate() {
                //01234567890123456789012345678901234567890123456
                // section | best  | current       | section
                // --------|-------|---------------|--------------
                // name    | --:-- | --:-- (--:--) | --:-- (--:--)
                let name_x = 1;
                let best_x = 11;
                let total_x = 19;
                let deltat_x = 25;
                let section_x = 35;
                let deltas_x = 41;

                let y = i as i32 + 3;

                engine.print(name_x, y, &s.name);
                engine.print(best_x - 2, y, "|");
                engine.print(best_x, y, &app.pb_total_time(i));
                engine.print(total_x - 2, y, "|");
                engine.print(total_x, y, &app.current_total_time(i));
                {
                    let time = app.delta_total_time(i);
                    engine.print_fbg(
                        deltat_x,
                        y,
                        &app.delta_time_to_string(i, time),
                        time.map_or(
                            Color::Reset,
                            |t| if t < 0 { Color::Blue } else { Color::Red },
                        ),
                        Color::Reset,
                    );
                }
                engine.print(section_x - 2, y, "|");
                engine.print(section_x, y, &app.current_section_time(i));
                {
                    let time = app.delta_section_time(i);
                    engine.print_fbg(
                        deltas_x,
                        y,
                        &app.delta_time_to_string(i, time),
                        time.map_or(
                            Color::Reset,
                            |t| if t < 0 { Color::Blue } else { Color::Red },
                        ),
                        Color::Reset,
                    );
                }
            }
            engine.draw();

            if engine.is_key_pressed(KeyCode::Char('q')) {
                break; // exits app
            }
        }

        Ok(())
    }

    fn update_current_time(&mut self) {
        if !self.running {
            return;
        }

        if self.current_section >= self.sections.len() {
            return;
        }

        self.sections[self.current_section].current_total =
            Some(self.start_time.elapsed().as_millis() as u32);
    }

    fn current_total_time(&self, section: usize) -> String {
        let s = &self.sections[section];
        self.time_to_string(section, s.current_total)
    }

    fn pb_total_time(&self, section: usize) -> String {
        let s = &self.sections[section];
        self.fixed_time_to_string(s.pb_total)
    }

    fn current_section_time(&self, section: usize) -> String {
        if section == 0 {
            return self.current_total_time(section);
        }

        let s = &self.sections[section];
        let last = &self.sections[section - 1];

        let time = if let (Some(c), Some(l)) = (s.current_total, last.current_total) {
            Some(c - l)
        } else {
            None
        };

        self.time_to_string(section, time)
    }

    fn _pb_section_time(&self, section: usize) -> String {
        if section == 0 {
            return self.pb_total_time(section);
        }

        let s = &self.sections[section];
        let last = &self.sections[section - 1];

        let time = if let (Some(c), Some(l)) = (s.pb_total, last.pb_total) {
            Some(c - l)
        } else {
            None
        };

        self.fixed_time_to_string(time)
    }

    fn delta_total_time(&self, section: usize) -> Option<i32> {
        let s = &self.sections[section];
        let time = if let (Some(p), Some(c)) = (s.pb_total, s.current_total) {
            Some(c as i32 - p as i32)
        } else {
            None
        };

        time
    }

    fn delta_section_time(&self, section: usize) -> Option<i32> {
        if section == 0 {
            return self.delta_total_time(section);
        }

        let s = &self.sections[section];
        let last = &self.sections[section - 1];

        let time = if let (Some(pb_c), Some(pb_l), Some(c_c), Some(c_l)) = (
            s.pb_total,
            last.pb_total,
            s.current_total,
            last.current_total,
        ) {
            Some((c_c - c_l) as i32 - (pb_c - pb_l) as i32)
        } else {
            None
        };

        time
    }

    fn time_to_string(&self, section: usize, time: Option<u32>) -> String {
        if let Some(t) = time {
            format!("{:>2}:{:02}", t / 60000, (t / 1000) % 60)
        } else {
            if section < self.current_section {
                "--:--".to_owned()
            } else {
                "     ".to_owned()
            }
        }
    }

    fn fixed_time_to_string(&self, time: Option<u32>) -> String {
        if let Some(t) = time {
            format!("{:>2}:{:02}", t / 60000, (t / 1000) % 60)
        } else {
            "--:--".to_owned()
        }
    }

    fn delta_time_to_string(&self, section: usize, time: Option<i32>) -> String {
        if let Some(t) = time {
            if t < 0 {
                let t = -t;
                format!("(-{}:{:02})", t / 60000, (t / 1000) % 60)
            } else {
                format!("(+{}:{:02})", t / 60000, (t / 1000) % 60)
            }
        } else {
            if section < self.current_section {
                "(--:--)".to_owned()
            } else {
                "       ".to_owned()
            }
        }
    }

    fn load_pb(game: &str) -> Result<Self> {
        let dirs = directories::ProjectDirs::from("", "", "speedy")
            .ok_or(anyhow!("No home directory found"))?;
        let data_dir = dirs.data_dir();
        let game_dir = data_dir.join(&game);
        let pb_file_path = game_dir.join("pb");
        let pb_file = File::open(pb_file_path).context("Failed to open pb file")?;
        Ok(ron::de::from_reader(pb_file)?)
    }

    fn save(&self) -> Result<()> {
        let dirs = directories::ProjectDirs::from("", "", "speedy")
            .ok_or(anyhow!("No home directory found"))?;
        let data_dir = dirs.data_dir();
        let game_dir = data_dir.join(&self.game);
        std::fs::create_dir_all(&game_dir)?;
        let name = self.start_date.format("%Y-%m-%dT%H:%M:%S.ron").to_string();
        let file_path = game_dir.join(name);
        let file = File::create(file_path)?;
        ron::ser::to_writer_pretty(file, self, ron::ser::PrettyConfig::default())?;

        let s = self.sections.last().ok_or(anyhow!(""))?;
        let new_pb = match (s.pb_total, s.current_total) {
            (Some(p), Some(c)) => c < p,
            (None, Some(_)) => true,
            _ => false,
        };

        if new_pb {
            let pb_file_path = game_dir.join("pb");
            let pb_file = File::create(pb_file_path)?;
            ron::ser::to_writer_pretty(pb_file, self, ron::ser::PrettyConfig::default())?;
        }

        Ok(())
    }
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about=None)]
#[command(propagate_version = true)]
struct Args {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand, Debug)]
enum Mode {
    Run {
        game: String,
    },
    Against {
        enemy: Option<String>,
    },
    Games,
    List {
        game: String,
    },
    Show {
        game: String,
        run: Option<String>,
    },
    Compare {
        game: String,
        a: Option<String>,
        b: Option<String>,
    },
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.mode {
        Mode::Run { game } => {
            let app = Arc::new(RwLock::new(App::load_pb(&game)?));
            App::spawn_signal_handler(Arc::clone(&app))?;
            App::launch_ui(&app)?;
        }
        _ => {
            eprintln!("Mode is not implemented yet!");
        }
    }

    Ok(())
}
