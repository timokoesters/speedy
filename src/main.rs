use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use console_engine::KeyCode;
use signal_hook::{consts::SIGUSR1, iterator::Signals};

struct Section {
    name: String,
    pb_total: Option<u32>,
    current_total: Option<u32>,
}

struct App {
    // name, pb and current time in sections
    sections: Vec<Section>,
    current_section: usize,
    start_time: Instant,
    running: bool,
}

impl App {
    fn load_portal() -> Self {
        let sections = vec![
            "1", "3", "5", "7", "9", "10", "12", "13", "14", "15", "16", "17", "18", "19", "e0",
            "e1", "e2",
        ];

        App {
            sections: sections
                .iter()
                .map(|&s| Section {
                    name: s.to_owned(),
                    pb_total: None,
                    current_total: None,
                })
                .collect(),
            current_section: 0,
            start_time: Instant::now(), // This is not the actual start, it will be reset later
            running: false,
        }
    }

    fn update_current_time(&mut self) {
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
        self.fixed_time_to_string(section, s.pb_total)
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

    fn pb_section_time(&self, section: usize) -> String {
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

        self.fixed_time_to_string(section, time)
    }

    fn delta_total_time(&self, section: usize) -> String {
        let s = &self.sections[section];
        let time = if let (Some(p), Some(c)) = (s.pb_total, s.current_total) {
            Some(c as i32 - p as i32)
        } else {
            None
        };

        self.delta_time_to_string(section, time)
    }

    fn delta_section_time(&self, section: usize) -> String {
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
            Some((pb_c - pb_l) as i32 - (c_c - c_l) as i32)
        } else {
            None
        };

        self.delta_time_to_string(section, time)
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

    fn fixed_time_to_string(&self, section: usize, time: Option<u32>) -> String {
        if let Some(t) = time {
            format!("{:>2}:{:02}", t / 60000, (t / 1000) % 60)
        } else {
            "--:--".to_owned()
        }
    }

    fn delta_time_to_string(&self, section: usize, time: Option<i32>) -> String {
        if let Some(t) = time {
            let sign = if t < 0 { '-' } else { '+' };
            format!("({}{}:{:02})", sign, t / 60000, (t / 1000) % 60)
        } else {
            if section < self.current_section {
                "(--:--)".to_owned()
            } else {
                "       ".to_owned()
            }
        }
    }
}

fn main() {
    let app = Arc::new(RwLock::new(App::load_portal()));

    let app2 = Arc::clone(&app);
    let mut signals = Signals::new(&[SIGUSR1]).unwrap();
    std::thread::spawn(move || {
        for sig in signals.forever() {
            if sig == SIGUSR1 {
                let app = &mut app2.write().unwrap();

                if app.current_section >= app.sections.len() {
                    continue;
                }

                let current_section = app.current_section;
                app.sections[current_section].current_total =
                    Some(app.start_time.elapsed().as_millis() as u32);
                app.current_section += 1;
            }
        }
    });

    let mut engine = console_engine::ConsoleEngine::init(45, 20, 10).unwrap();
    loop {
        engine.wait_frame();
        engine.clear_screen();

        engine.print(0, 0, "speedy: Portal1\nsection best  current      total");
        let app = &mut app.write().unwrap();
        app.update_current_time();

        for (i, s) in app.sections.iter().enumerate() {
            engine.print(
                0,
                i as i32 + 2,
                &format!(
                    "{:8}{}  {} {}  {}  {}",
                    s.name,
                    app.pb_total_time(i),
                    app.current_section_time(i),
                    app.delta_section_time(i),
                    app.current_total_time(i),
                    app.delta_total_time(i),
                ),
            );
        }
        engine.draw();

        if engine.is_key_pressed(KeyCode::Char('q')) {
            break; // exits app
        }
    }
}
