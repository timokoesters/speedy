use anyhow::{anyhow, ensure, Context, Result};
use clap::{Parser, Subcommand};
use console_engine::crossterm::terminal;
use console_engine::pixel::pxl_bg;
use std::fs::File;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use console_engine::{Color, ConsoleEngine, KeyCode};
use rodio::source::SineWave;
use rodio::{Sink, Source};
use serde::{Deserialize, Serialize};
use signal_hook::{consts::SIGUSR1, iterator::Signals};

const FG: Color = Color::Rgb {
    r: 0xf3,
    g: 0xf2,
    b: 0xcc,
};
const GREY: Color = Color::Rgb {
    r: 0x62,
    g: 0x62,
    b: 0x62,
};
const RED: Color = Color::Rgb {
    r: 0xf0,
    g: 0x5e,
    b: 0x48,
};
const BLUE: Color = Color::Rgb {
    r: 0x7c,
    g: 0xaf,
    b: 0xc2,
};
const GOLD: Color = Color::Rgb {
    r: 0xfa,
    g: 0xd5,
    b: 0x66,
};
const BG: Color = Color::Rgb {
    r: 0x09,
    g: 0x09,
    b: 0x09,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Section {
    name: String,
    #[serde(skip_serializing, rename(deserialize = "time"))]
    pb_total: Option<u32>,
    #[serde(skip)]
    sum_of_best_total: Option<u32>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        skip_deserializing,
        rename(serialize = "time")
    )]
    current_total: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct App {
    game: String,
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
        let size = terminal::size()?;
        ensure!(size.0 >= 49);
        ensure!(size.1 >= app.read().unwrap().sections.len() as u16 + 3);
        let mut engine = ConsoleEngine::init(size.0 as u32, size.1 as u32, 10)?;
        loop {
            engine.wait_frame();
            engine.fill(pxl_bg(' ', BG));

            let app = &mut app.write().expect("RwLock not poisoned");
            app.update_current_time();

            engine.print_fbg(0, 0, &format!(" speedy: {}", app.game), FG, BG);
            engine.print_fbg(
                0,
                1,
                " section | best  | current       | section      ",
                FG,
                BG,
            );
            engine.print_fbg(
                0,
                2,
                " --------|-------|---------------|--------------",
                FG,
                BG,
            );
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

                engine.print_fbg(name_x, y, &s.name, FG, BG);
                engine.print_fbg(best_x - 2, y, "|", FG, BG);
                engine.print_fbg(best_x, y, &app.pb_total_time(i), FG, BG);
                engine.print_fbg(total_x - 2, y, "|", FG, BG);
                app.current_total_time(i, &mut engine, total_x, y)?;
                {
                    app.delta_total_time(i, &mut engine, deltat_x, y)?;
                }
                engine.print_fbg(section_x - 2, y, "|", FG, BG);
                app.current_section_time(i, &mut engine, section_x, y)?;
                app.delta_section_time(i, &mut engine, deltas_x, y)?;
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

    fn current_total_time(
        &self,
        section: usize,
        engine: &mut ConsoleEngine,
        x: i32,
        y: i32,
    ) -> Result<()> {
        if let Some(s) = self.sections[section].current_total {
            engine.print_fbg(x, y, &self.time_to_string(0, Some(s)), FG, BG);
            return Ok(());
        }

        if let Some(s) = self.sections[section].sum_of_best_total {
            engine.print_fbg(
                x,
                y,
                &self.time_to_string(0, Some((s as i32 + self.loss_so_far()) as u32)),
                GREY,
                BG,
            );
            return Ok(());
        }

        return Ok(());
    }

    fn pb_total_time(&self, section: usize) -> String {
        let s = &self.sections[section];
        self.fixed_time_to_string(s.pb_total)
    }

    fn current_section_time(
        &self,
        section: usize,
        engine: &mut ConsoleEngine,
        x: i32,
        y: i32,
    ) -> Result<()> {
        let s = &self.sections[section];
        let last = if section > 0 {
            Some(&self.sections[section - 1])
        } else {
            None
        };

        let sum_of_best_section;
        if let (Some(c), Some(l)) = (
            s.sum_of_best_total,
            last.map_or(Some(0), |l| l.sum_of_best_total),
        ) {
            sum_of_best_section = Some(c - l);
        } else {
            sum_of_best_section = None;
        }

        if let (Some(c), Some(l)) = (s.current_total, last.map_or(Some(0), |l| l.current_total)) {
            let time = c - l;
            engine.print_fbg(
                x,
                y,
                &self.time_to_string(section, Some(time)),
                if section < self.current_section && Some(time) < sum_of_best_section {
                    GOLD
                } else {
                    FG
                },
                BG,
            );
            return Ok(());
        }

        if let Some(s) = sum_of_best_section {
            engine.print_fbg(x, y, &self.time_to_string(0, Some(s)), GREY, BG);
            return Ok(());
        }

        // Print nothing
        Ok(())
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

    fn last_loss(&self) -> i32 {
        if self.current_section == 0 {
            return 0;
        } else if let (Some(l_c), Some(last_sob)) = (
            self.sections[self.current_section - 1].current_total,
            self.sections[self.current_section - 1].sum_of_best_total,
        ) {
            return l_c as i32 - last_sob as i32;
        }

        0
    }

    fn loss_so_far(&self) -> i32 {
        if let (Some(c), Some(s_c)) = (
            self.sections[self.current_section].current_total,
            self.sections[self.current_section].sum_of_best_total,
        ) {
            let last_loss = self.last_loss();
            if c > (s_c as i32 + last_loss) as u32 {
                return c as i32 - s_c as i32;
            } else {
                return last_loss;
            }
        }
        0
    }

    fn delta_total_time(
        &self,
        section: usize,
        engine: &mut ConsoleEngine,
        x: i32,
        y: i32,
    ) -> Result<()> {
        let s = &self.sections[section];

        if let (Some(p), Some(c)) = (s.pb_total, s.current_total) {
            let delta = c as i32 - p as i32;

            if section == self.current_section {
                if let Some(s_c) = s.sum_of_best_total {
                    if c < (s_c as i32 + self.loss_so_far()) as u32 {
                        engine.print_fbg(
                            x,
                            y,
                            &("/".to_owned()
                                + &self.time_to_string(
                                    section,
                                    Some((s_c as i32 + self.loss_so_far()) as u32),
                                )),
                            GREY,
                            BG,
                        );
                        return Ok(());
                    }
                }
            }

            engine.print_fbg(
                x,
                y,
                &self.delta_time_to_string(section, Some(delta)),
                if delta < 0 { BLUE } else { RED },
                BG,
            );

            return Ok(());
        }

        // Print nothing
        Ok(())
    }

    fn delta_section_time(
        &self,
        section: usize,
        engine: &mut ConsoleEngine,
        x: i32,
        y: i32,
    ) -> Result<()> {
        let s = &self.sections[section];

        let last = if section > 0 {
            Some(&self.sections[section - 1])
        } else {
            None
        };

        if let (Some(pb_c), Some(pb_l), Some(c_c), Some(c_l)) = (
            s.pb_total,
            last.map_or(Some(0), |l| l.pb_total),
            s.current_total,
            last.map_or(Some(0), |l| l.current_total),
        ) {
            let section_time = c_c - c_l;
            let pb_section_time = pb_c - pb_l;
            let delta = section_time as i32 - pb_section_time as i32;

            if section == self.current_section {
                if let (Some(s_c), Some(s_l)) = (
                    s.sum_of_best_total,
                    last.map_or(Some(0), |l| l.sum_of_best_total),
                ) {
                    let sum_of_best_time = s_c - s_l;
                    if section_time < sum_of_best_time {
                        engine.print_fbg(
                            x,
                            y,
                            &("/".to_owned()
                                + &self.time_to_string(section, Some(sum_of_best_time))),
                            GREY,
                            BG,
                        );
                        return Ok(());
                    }
                }
            }

            engine.print_fbg(
                x,
                y,
                &self.delta_time_to_string(section, Some(delta)),
                if delta < 0 { BLUE } else { RED },
                BG,
            );

            return Ok(());
        };

        // Print nothing
        Ok(())
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

    fn load_default(game: &str) -> Result<Self> {
        let dirs = directories::ProjectDirs::from("", "", "speedy")
            .ok_or(anyhow!("No home directory found"))?;
        let data_dir = dirs.data_dir();
        let game_dir = data_dir.join(&game);
        let pb_file_path = game_dir.join("pb.ron");
        let pb_file = File::open(pb_file_path).context("Failed to open pb file")?;

        let sum_of_best_file_path = game_dir.join("sum_of_best.ron");
        let sum_of_best_file = File::open(sum_of_best_file_path);

        let mut app: Self = ron::de::from_reader(pb_file)?;
        if let Ok(sum_of_best_file) = sum_of_best_file {
            let sum_of_best: Self = ron::de::from_reader(sum_of_best_file)?;

            ensure!(sum_of_best.sections.len() == app.sections.len());
            for i in 0..app.sections.len() {
                ensure!(sum_of_best.sections[i].name == app.sections[i].name);
                app.sections[i].sum_of_best_total = sum_of_best.sections[i].pb_total;
            }
        }
        Ok(app)
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
            let pb_file_path = game_dir.join("pb.ron");
            let pb_file = File::create(pb_file_path)?;
            ron::ser::to_writer_pretty(pb_file, self, ron::ser::PrettyConfig::default())?;
        }

        let mut app_clone = self.clone();
        let mut new_sum_of_best = 0;
        for i in 0..self.sections.len() {
            let mut section_time = self.sections[i]
                .current_total
                .expect("we just did a full run");
            let mut sob_time = self.sections[i].sum_of_best_total;
            if i > 0 {
                section_time -= self.sections[i - 1]
                    .current_total
                    .expect("we just did a full run");
                sob_time = sob_time.map(|sob| {
                    sob - self.sections[i - 1]
                        .sum_of_best_total
                        .expect("sum of best is a full run")
                });
            }

            if sob_time < Some(section_time) {
                new_sum_of_best += sob_time.expect("comparison above worked");
            } else {
                new_sum_of_best += section_time;
            }
            app_clone.sections[i].current_total = Some(new_sum_of_best);
        }

        let sob_file_path = game_dir.join("sum_of_best.ron");
        let sob_file = File::create(sob_file_path)?;
        ron::ser::to_writer_pretty(sob_file, &app_clone, ron::ser::PrettyConfig::default())?;

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
            let app = Arc::new(RwLock::new(App::load_default(&game)?));
            App::spawn_signal_handler(Arc::clone(&app))?;
            App::launch_ui(&app)?;
        }
        _ => {
            eprintln!("Mode is not implemented yet!");
        }
    }

    Ok(())
}
