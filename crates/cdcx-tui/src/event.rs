use crossterm::event::{self, Event as CrosstermEvent, KeyEvent, KeyEventKind, MouseEvent};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Tick,
    Resize(u16, u16),
}

pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Event>,
}

impl EventHandler {
    pub fn new(tick_rate_ms: u64) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let tick_rate = Duration::from_millis(tick_rate_ms);

        // Crossterm event polling must run in a real OS thread (it blocks)
        std::thread::spawn(move || {
            let mut last_tick = std::time::Instant::now();
            loop {
                let timeout = tick_rate.saturating_sub(last_tick.elapsed());
                if event::poll(timeout).unwrap_or(false) {
                    match event::read() {
                        Ok(CrosstermEvent::Key(key))
                            if key.kind == KeyEventKind::Press
                                && tx.send(Event::Key(key)).is_err() =>
                        {
                            return;
                        }
                        Ok(CrosstermEvent::Mouse(mouse)) => {
                            let _ = tx.send(Event::Mouse(mouse));
                        }
                        Ok(CrosstermEvent::Resize(w, h)) => {
                            let _ = tx.send(Event::Resize(w, h));
                        }
                        _ => {}
                    }
                }
                if last_tick.elapsed() >= tick_rate {
                    if tx.send(Event::Tick).is_err() {
                        return;
                    }
                    last_tick = std::time::Instant::now();
                }
            }
        });

        Self { rx }
    }

    pub async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
}
