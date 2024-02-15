use anyhow::{anyhow, bail, ensure, Context, Result};
use clap::{Parser, Subcommand};
use console_engine::crossterm::terminal;
use console_engine::pixel::pxl_bg;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use regex::Regex;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
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

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GameConfig {
    version: u32,

    #[serde(skip)]
    directory_name: String,

    full_game_name: String,
    bridge_script: Option<PathBuf>,
    sections: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Section {
    name: String,
    time: u32,
}

#[derive(Debug, Clone)]
struct RunApp {
    config: GameConfig,
    current_sections: Vec<Section>,
    pb_sections: Option<Vec<Section>>,
    sum_of_best_sections: Option<Vec<Section>>,
    start_time: Instant,
    start_date: chrono::DateTime<chrono::Local>,
    running: bool,
    bridge_error: bool,
}

impl RunApp {
    fn handle_signal(app: &RwLock<Self>, sink: &Sink, sig: i32) -> Result<()> {
        if sig != SIGUSR1 {
            return Ok(());
        }

        let app = &mut app.write().expect("RwLock not poisoned");

        if !app.running && app.current_sections.len() == 0 {
            app.running = true;
            app.start_time = Instant::now();
            app.start_date = chrono::Local::now();

            let name = app.config.sections[0].clone();
            app.current_sections.push(Section { name, time: 0 });

            let source = SineWave::new(1.5 * 440.0)
                .take_duration(Duration::from_secs_f32(0.1))
                .amplify(0.20);
            sink.append(source.clone());

            return Ok(());
        }

        if !app.running {
            return Ok(());
        }

        app.update_current_time();

        let source = SineWave::new(440.0)
            .take_duration(Duration::from_secs_f32(0.1))
            .amplify(0.20);
        sink.append(source.clone());

        if app.current_sections.len() >= app.config.sections.len() {
            app.running = false;
            // Run finished
            app.save()?;

            let source = SineWave::new(0.5 * 440.0)
                .take_duration(Duration::from_secs_f32(0.5))
                .amplify(0.20);
            sink.append(source.clone());

            return Ok(());
        }

        let name = app.config.sections[app.current_sections.len()].clone();
        let time = app.start_time.elapsed().as_millis() as u32;
        app.current_sections.push(Section { name, time });

        Ok(())
    }

    fn spawn_signal_handler(app: Arc<RwLock<Self>>) -> Result<()> {
        let mut signals = Signals::new(&[SIGUSR1])?;
        let (stream, audio_stream_handle) = rodio::OutputStream::try_default()?;
        let sink = Sink::try_new(&audio_stream_handle)?;

        // Keep stream alive forever
        Box::leak(Box::new(stream));

        std::thread::spawn(move || {
            for sig in signals.forever() {
                Self::handle_signal(&app, &sink, sig)?;
            }

            Ok::<_, anyhow::Error>(())
        });

        Ok(())
    }

    fn spawn_bridge_handler(app: Arc<RwLock<Self>>) -> Result<Option<Child>> {
        let script = app.read().unwrap().config.bridge_script.clone();
        if let Some(script) = script {
            let child = Command::new(script)
                .stdout(std::io::stderr())
                .spawn()
                .unwrap();
            return Ok(Some(child));
        }

        Ok(None)
    }

    fn launch_ui(app: &RwLock<Self>) -> Result<()> {
        let size = terminal::size()?;
        ensure!(size.0 >= 49);
        ensure!(size.1 >= app.read().unwrap().config.sections.len() as u16 + 3);
        let mut engine = ConsoleEngine::init(size.0 as u32, size.1 as u32, 10)?;
        loop {
            engine.wait_frame();

            let app = &mut app.write().expect("RwLock not poisoned");
            app.update_current_time();

            if app.bridge_error {
                bail!("Bridge error!");
            }

            engine.fill(pxl_bg(' ', BG));
            engine.print_fbg(
                0,
                0,
                &format!(" speedy: {}", app.config.full_game_name),
                FG,
                BG,
            );
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
            for (i, section_name) in app.config.sections.iter().enumerate() {
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

                engine.print_fbg(name_x, y, &section_name, FG, BG);
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

        if self.current_sections.len() > self.config.sections.len() {
            return;
        }

        self.current_sections.last_mut().unwrap().time =
            self.start_time.elapsed().as_millis() as u32;
    }

    fn current_total_time(
        &self,
        section: usize,
        engine: &mut ConsoleEngine,
        x: i32,
        y: i32,
    ) -> Result<()> {
        if let Some(s) = self.current_sections.get(section) {
            engine.print_fbg(x, y, &self.time_to_string(0, Some(s.time)), FG, BG);
            return Ok(());
        }

        if let Some(s) = &self.sum_of_best_sections {
            engine.print_fbg(
                x,
                y,
                &self.time_to_string(
                    0,
                    Some((s[section].time as i32 + self.loss_so_far()) as u32),
                ),
                GREY,
                BG,
            );
            return Ok(());
        }

        return Ok(());
    }

    fn pb_total_time(&self, section: usize) -> String {
        self.fixed_time_to_string(self.pb_sections.as_ref().map(|s| s[section].time))
    }

    fn current_section_time(
        &self,
        section: usize,
        engine: &mut ConsoleEngine,
        x: i32,
        y: i32,
    ) -> Result<()> {
        let sob_section;
        if let Some(sum_of_best_sections) = &self.sum_of_best_sections {
            if section == 0 {
                sob_section = Some(sum_of_best_sections[section].time);
            } else {
                sob_section = Some(
                    sum_of_best_sections[section].time - sum_of_best_sections[section - 1].time,
                );
            }
        } else {
            sob_section = None;
        }

        if let Some(c) = self.current_sections.get(section).map(|s| s.time) {
            let last_time;
            if section == 0 {
                last_time = 0;
            } else {
                last_time = self.current_sections[section - 1].time
            }
            let time = c - last_time;
            engine.print_fbg(
                x,
                y,
                &self.time_to_string(section, Some(time)),
                if section < self.current_sections.len() - 1 && Some(time) < sob_section {
                    GOLD
                } else {
                    FG
                },
                BG,
            );
            return Ok(());
        }

        if let Some(s) = sob_section {
            engine.print_fbg(x, y, &self.time_to_string(0, Some(s)), GREY, BG);
            return Ok(());
        }

        // Print nothing
        Ok(())
    }

    fn last_loss(&self) -> i32 {
        if self.current_sections.len() <= 1 {
            return 0;
        }

        let current = self.current_sections[self.current_sections.len() - 2].time;
        if let Some(sum_of_best_sections) = &self.sum_of_best_sections {
            let sob = sum_of_best_sections[self.current_sections.len() - 2].time;

            return current as i32 - sob as i32;
        }

        return 0;
    }

    fn loss_so_far(&self) -> i32 {
        if self.current_sections.len() == 0 {
            return 0;
        }

        if let Some(sum_of_best_sections) = &self.sum_of_best_sections {
            let c = self.current_sections.last().unwrap().time;
            let s_c = sum_of_best_sections[self.current_sections.len() - 1].time;
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
        if let (Some(c), Some(pb_sections)) =
            (self.current_sections.get(section), &self.pb_sections)
        {
            let p = &pb_sections[section];
            let delta = c.time as i32 - p.time as i32;

            if section == self.current_sections.len() - 1 {
                if let Some(sum_of_best_sections) = &self.sum_of_best_sections {
                    let s_c = sum_of_best_sections[section].time;
                    if c.time < (s_c as i32 + self.loss_so_far()) as u32 {
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
        if self.current_sections.len() < section + 1 {
            // Print nothing
            return Ok(());
        }

        if let Some(pb_sections) = &self.pb_sections {
            let pb_c = pb_sections[section].time;
            let pb_l = if section == 0 {
                0
            } else {
                pb_sections[section - 1].time
            };
            let c_c = self.current_sections[section].time;
            let c_l = if section == 0 {
                0
            } else {
                self.current_sections[section - 1].time
            };

            let section_time = c_c - c_l;
            let pb_section_time = pb_c - pb_l;
            let delta = section_time as i32 - pb_section_time as i32;

            if section == self.current_sections.len() - 1 {
                if let Some(sum_of_best_sections) = &self.sum_of_best_sections {
                    let s_c = sum_of_best_sections[section].time;
                    let s_l = if section == 0 {
                        0
                    } else {
                        sum_of_best_sections[section - 1].time
                    };

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
            if section < self.current_sections.len() - 1 {
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
            if section < self.current_sections.len() - 1 {
                "(--:--)".to_owned()
            } else {
                "       ".to_owned()
            }
        }
    }

    fn prepare_run(config: GameConfig) -> Result<Self> {
        let sum_of_best = load_run(&config.directory_name, "sum_of_best.run")?;

        if let Some(sum_of_best) = &sum_of_best {
            ensure!(config.sections.len() == sum_of_best.len());
            for i in 0..config.sections.len() {
                ensure!(config.sections[i] == sum_of_best[i].name);
            }
        }

        Ok(Self {
            config,
            current_sections: Vec::new(),
            pb_sections: None,
            sum_of_best_sections: sum_of_best,
            start_time: Instant::now(),
            start_date: chrono::Local::now(),
            running: false,
            bridge_error: false,
        })
    }

    fn set_pb(&mut self, pb: Vec<Section>) -> Result<()> {
        ensure!(self.config.sections.len() == pb.len());
        for i in 0..self.config.sections.len() {
            ensure!(self.config.sections[i] == pb[i].name);
        }

        self.pb_sections = Some(pb);

        Ok(())
    }

    fn save(&self) -> Result<()> {
        let name = self.start_date.format("%Y-%m-%dT%H:%M:%S.run").to_string();
        save_run(&self.config.directory_name, &name, &self.current_sections)?;

        let new_pb;
        if let Some(pb) = &self.pb_sections {
            ensure!(pb.len() == self.current_sections.len());
            for i in 0..pb.len() {
                ensure!(pb[i].name == self.current_sections[i].name);
            }

            new_pb = self
                .current_sections
                .last()
                .context("empty current run")?
                .time
                < pb.last().context("empty pb run")?.time;
        } else {
            new_pb = true;
        }

        if new_pb {
            save_run(
                &self.config.directory_name,
                "pb.run",
                &self.current_sections,
            )?;
        }

        let mut new_sob = Vec::new();
        if let Some(sum_of_best_sections) = &self.sum_of_best_sections {
            let mut new_sum_of_best = 0;
            for i in 0..self.current_sections.len() {
                let mut section_time = self.current_sections[i].time;
                let mut sob_time = sum_of_best_sections[i].time;
                if i > 0 {
                    section_time -= self.current_sections[i - 1].time;
                    sob_time -= sum_of_best_sections[i - 1].time;
                }

                if sob_time < section_time {
                    new_sum_of_best += sob_time;
                } else {
                    new_sum_of_best += section_time;
                }
                new_sob.push(Section {
                    name: self.current_sections[i].name.clone(),
                    time: new_sum_of_best,
                });
            }
        } else {
            new_sob = self.current_sections.clone();
        }

        save_run(&self.config.directory_name, "sum_of_best.run", &new_sob)?;

        Ok(())
    }
}

fn min_sec_mil_to_millis(min: u32, sec: u32, mil: u32) -> u32 {
    (min * 60 + sec) * 1000 + mil
}

fn millis_to_min_sec_mil(millis: u32) -> (u32, u32, u32) {
    let min = millis / 60000;
    let sec = (millis / 1000) % 60;
    let mil = millis % 1000;
    (min, sec, mil)
}

fn load_config(game: &str) -> Result<GameConfig> {
    let dirs = directories::ProjectDirs::from("", "", "speedy")
        .ok_or(anyhow!("No home directory found"))?;
    let data_dir = dirs.data_dir();
    let game_dir = data_dir.join(game);
    let config_path = game_dir.join("config.toml");
    let config_str = fs::read_to_string(config_path)?;
    let mut config: GameConfig = toml::from_str(&config_str)?;
    config.directory_name = game.to_owned();

    ensure!(config.sections.len() > 0);

    Ok(config)
}

fn load_all_configs() -> Result<Vec<GameConfig>> {
    let dirs = directories::ProjectDirs::from("", "", "speedy")
        .ok_or(anyhow!("No home directory found"))?;
    let data_dir = dirs.data_dir();
    let mut results = Vec::new();
    for game_dir in fs::read_dir(&data_dir)? {
        let game = game_dir?
            .file_name()
            .into_string()
            .ok()
            .context("Invalid OsString")?;
        if let Ok(config) = load_config(&game) {
            results.push(config);
        }
    }

    Ok(results)
}

fn write_config(config: &GameConfig) -> Result<()> {
    let dirs = directories::ProjectDirs::from("", "", "speedy")
        .ok_or(anyhow!("No home directory found"))?;
    let data_dir = dirs.data_dir();
    let game_dir = data_dir.join(&config.directory_name);

    std::fs::create_dir_all(&game_dir)?;

    let config_str = toml::to_string_pretty(config)?;
    let config_path = game_dir.join("config.toml");
    fs::write(config_path, &config_str)?;

    Ok(())
}

fn load_run(game: &str, run: &str) -> Result<Option<Vec<Section>>> {
    let dirs = directories::ProjectDirs::from("", "", "speedy")
        .ok_or(anyhow!("No home directory found"))?;
    let data_dir = dirs.data_dir();
    let game_dir = data_dir.join(&game);
    let file_path = game_dir.join(run);

    let file = if let Ok(file) = File::open(file_path) {
        file
    } else {
        return Ok(None);
    };

    let file = BufReader::new(file);

    let mut sections = Vec::new();
    for line in file.lines() {
        let line = line.context("Failed to read line in run file")?;

        // Lines look like this: "escape01: 20m01.212s
        let re = Regex::new(r"^(.*): (\d*)m(\d{2})\.(\d{3})s$").unwrap();
        let cap = re.captures(&line).context("Invalid run file")?;

        let section_name = cap[1].to_owned();
        let section_time_ms = min_sec_mil_to_millis(
            cap[2].parse().unwrap(),
            cap[3].parse().unwrap(),
            cap[4].parse().unwrap(),
        );

        sections.push(Section {
            name: section_name,
            time: section_time_ms,
        });
    }

    Ok(Some(sections))
}

fn save_run(game: &str, run: &str, sections: &[Section]) -> Result<()> {
    let dirs = directories::ProjectDirs::from("", "", "speedy")
        .ok_or(anyhow!("No home directory found"))?;
    let data_dir = dirs.data_dir();
    let game_dir = data_dir.join(game);

    let file_path = game_dir.join(run);
    let mut file = BufWriter::new(File::create(file_path)?);

    for section in sections {
        let (min, sec, mil) = millis_to_min_sec_mil(section.time);
        writeln!(file, "{}: {}m{:02}.{:03}s", section.name, min, sec, mil)?;
    }

    file.flush()?;

    Ok(())
}

fn ask(q: &str) -> Result<String> {
    print!("{}", q);

    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(input.trim().to_owned())
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
    ListGames,
    NewGame {
        game: String,
    },
    ListRuns {
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
            let mut app = RunApp::prepare_run(load_config(&game)?)?;

            if let Some(pb) = load_run(&game, "pb.run")? {
                app.set_pb(pb)?;
            }

            let app = Arc::new(RwLock::new(app));

            RunApp::spawn_signal_handler(Arc::clone(&app))?;
            let child = RunApp::spawn_bridge_handler(Arc::clone(&app))?;
            // child.unwrap().stdout.unwrap();
            RunApp::launch_ui(&app)?;
            if let Some(mut child) = child {
                child.kill().unwrap();
            }
        }
        Mode::NewGame { game } => {
            println!("Registering new game");
            let full_game_name = ask("Full game name: ")?;

            println!("Enter section names (CTRL-D or write empty line to stop)");
            let mut section_names = Vec::new();
            for i in 1.. {
                let name = ask(&format!("section{}: ", i))?;
                if name.is_empty() {
                    break;
                }
                section_names.push(name);
            }
            if section_names.is_empty() {
                println!("\nGame creation cancelled");
                return Ok(());
            }

            let bridge_script_raw = ask("\nOptional: Enter bridge script path: ")?;

            let bridge_script = if bridge_script_raw.is_empty() {
                None
            } else {
                Some(PathBuf::from(bridge_script_raw))
            };

            let ask_save = ask(&format!(
                "Do you want to create {} with {} sections? [Y/n]: ",
                game,
                section_names.len()
            ))?;

            if ["y", "yes", "ja", "j", ""].contains(&&*ask_save.to_lowercase()) {
                let config = GameConfig {
                    version: 1,
                    directory_name: game,
                    full_game_name,
                    bridge_script,
                    sections: section_names,
                };

                write_config(&config)?;

                println!("Done");
            } else {
                println!("Game creation cancelled");
            }
        }
        Mode::ListGames => {
            let configs = load_all_configs()?;
            if configs.is_empty() {
                println!("No games registered yet");
            } else {
                for config in configs {
                    let pb = if let Some(pb_run) = load_run(&config.directory_name, "pb.run")? {
                        let (min, sec, _mil) =
                            millis_to_min_sec_mil(pb_run.last().context("Run is empty")?.time);
                        format!("{}m{:02}s", min, sec)
                    } else {
                        "No PB!".to_owned()
                    };
                    println!(
                        "{}: [{}] {}",
                        pb, config.directory_name, config.full_game_name
                    );
                }
            }
        }
        _ => {
            eprintln!("Mode is not implemented yet!");
        }
    }

    Ok(())
}
