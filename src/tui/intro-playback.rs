//! Event-driven playback for the generated intro frame sequence.

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};

use super::intro::{
    self, FOX_FRAME_COUNT, FRAME_DURATION, FrameSelection, INTRO_MIN_COLUMNS, INTRO_MIN_ROWS,
    IntroPlan, IntroResolution, TUI_FADE_OPACITY,
};
use super::intro_render;
use super::model::AppState;
use super::render;
use super::terminal::{ProcessSignal, TerminalSession};

const MAX_EVENT_WAIT: Duration = Duration::from_millis(25);
const _: () = assert!(FOX_FRAME_COUNT == intro_render::FRAME_SEQUENCE.len());

pub(crate) enum PlaybackExit {
    Continue {
        resolution: Option<IntroResolution>,
        deferred: Vec<Event>,
    },
    Process(u8),
}

pub(crate) fn play(
    session: &mut TerminalSession,
    plan: IntroPlan,
    state: &AppState,
) -> io::Result<PlaybackExit> {
    if matches!(plan, IntroPlan::Suppressed) {
        return Ok(PlaybackExit::Continue {
            resolution: None,
            deferred: Vec::new(),
        });
    }

    let started = Instant::now();
    let mut rendered = None;
    let mut deferred = Vec::new();
    loop {
        if let Some(signal) = session.signals().take() {
            return Ok(PlaybackExit::Process(match signal {
                ProcessSignal::Interrupt => 130,
                ProcessSignal::Terminate => 1,
            }));
        }

        let elapsed = started.elapsed();
        let frame_index = match intro::select_frame(elapsed) {
            FrameSelection::Complete => {
                return Ok(PlaybackExit::Continue {
                    resolution: Some(IntroResolution::Completed),
                    deferred,
                });
            }
            FrameSelection::Frame(frame_index) => frame_index,
        };
        if rendered != Some(frame_index) {
            if frame_index < FOX_FRAME_COUNT {
                session.draw(|frame| {
                    intro_render::render(frame, frame_index, state.color_mode);
                })?;
            } else {
                let fade_index = frame_index - FOX_FRAME_COUNT;
                let opacity = TUI_FADE_OPACITY
                    .get(fade_index)
                    .copied()
                    .expect("selected interface fade frame is generated from its timing");
                session.draw(|frame| render::render_faded(frame, state, opacity))?;
            }
            rendered = Some(frame_index);
        }

        let next_frame = FRAME_DURATION
            * u32::try_from(frame_index + 1).expect("the intro frame count fits u32");
        let wait = next_frame
            .saturating_sub(started.elapsed())
            .min(MAX_EVENT_WAIT);
        if !event::poll(wait)? {
            continue;
        }

        let input = event::read()?;
        match input {
            Event::Resize(columns, rows) => {
                deferred.push(input);
                if columns < INTRO_MIN_COLUMNS || rows < INTRO_MIN_ROWS {
                    return Ok(PlaybackExit::Continue {
                        resolution: None,
                        deferred,
                    });
                }
            }
            Event::Key(key)
                if key.kind != KeyEventKind::Release
                    && intro::classify_key(key.code).consumed() =>
            {
                return Ok(PlaybackExit::Continue {
                    resolution: Some(IntroResolution::Skipped),
                    deferred,
                });
            }
            _ => deferred.push(input),
        }
    }
}
