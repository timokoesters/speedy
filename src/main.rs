use std::sync::{Arc, RwLock};
use std::time::Instant;

use console_engine::KeyCode;
use signal_hook::{consts::SIGUSR1, iterator::Signals};

struct App {
    // name, pb and current time in sections
    sections: Vec<(String, Option<u32>, Option<u32>)>,
    current_section: usize,
    start_time: Instant,
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
                .map(|&s| (s.to_owned(), None, None))
                .collect(),
            current_section: 0,
            start_time: Instant::now(), // This is not the actual start, it will be reset later
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
                app.sections[current_section].2 = Some(app.start_time.elapsed().as_millis() as u32);
                app.current_section += 1;
            }
        }
    });

    let mut engine = console_engine::ConsoleEngine::init(45, 20, 10).unwrap();
    loop {
        engine.wait_frame();
        engine.clear_screen();

        engine.print(0, 0, "speedy: Portal1\nsection best  delta  current total");
        let app = app.read().unwrap();
        for (i, (name, pb, mut current_total)) in app.sections.iter().enumerate() {
            let last_total = if i >= 1 {
                app.sections[i - 1].2.unwrap_or(0)
            } else {
                0
            };
            let last_pb_total = if i > 1 {
                app.sections[i - 1].1.unwrap_or(0)
            } else {
                0
            };

            if i == app.current_section {
                current_total = Some(app.start_time.elapsed().as_millis() as u32);
            }
            let delta_str = if let (Some(p), Some(c)) = (pb, current_total) {
                let d = c as i32 - *p as i32;
                let sign = if d < 0 { '-' } else { '+' };
                format!("{}{}:{:02}", sign, d / 60000, (d / 1000) % 60)
            } else {
                if i < app.current_section {
                    "-:--".to_owned()
                } else {
                    "".to_owned()
                }
            };

            let pb_str = pb.map_or("-:--".to_owned(), |p| {
                let p = p - last_pb_total;
                format!("{}:{:2}", p / 60000, (p / 1000) % 60)
            });

            let current_str;
            let current_total_str;

            if let Some(c) = current_total {
                let d = c - last_total;
                current_str = format!("{}:{:02}", d / 60000, (d / 1000) % 60);
                current_total_str = format!("{}:{:02}", c / 60000, (c / 1000) % 60);
            } else {
                if i < app.current_section {
                    current_str = "-:--".to_owned();
                    current_total_str = "-:--".to_owned();
                } else {
                    current_str = "".to_owned();
                    current_total_str = "".to_owned();
                }
            };

            engine.print(
                0,
                i as i32 + 2,
                &format!(
                    "{:8}{:6}{:8}{:7}{}",
                    name, pb_str, delta_str, current_str, current_total_str
                ),
            );
        }
        engine.draw();

        if engine.is_key_pressed(KeyCode::Char('q')) {
            break; // exits app
        }
    }
}
