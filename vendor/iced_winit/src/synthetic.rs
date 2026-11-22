//! JAMSDESIGN PATCH: synthetic pointer input.
//!
//! Lets an app feed pointer events that didn't come from winit, e.g. a drawing tablet read
//! through the Wayland tablet protocol (where the compositor stops sending pen input as mouse
//! events). Queued events go through the same path as real winit events on the next event
//! loop iteration, so hover, presses and the cursor position all behave normally.
//!
//! The loop must be woken to process them; sending any message to the app (e.g. through a
//! subscription) does that.

use std::sync::Mutex;

/// A pointer event in window coordinates (logical pixels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pointer {
    /// The pointer moved to this position.
    Moved {
        /// Logical x.
        x: f32,
        /// Logical y.
        y: f32,
    },
    /// The primary button (pen tip) went down.
    Pressed,
    /// The primary button (pen tip) went up.
    Released,
    /// The pointer left the window.
    Left,
}

pub(crate) static QUEUE: Mutex<Vec<Pointer>> = Mutex::new(Vec::new());

/// Queues pointer events for the application's (first) window.
pub fn push(events: impl IntoIterator<Item = Pointer>) {
    QUEUE.lock().expect("synthetic input queue").extend(events);
}

pub(crate) fn drain() -> Vec<Pointer> {
    std::mem::take(&mut *QUEUE.lock().expect("synthetic input queue"))
}

/// Builds the winit event for a synthetic pointer event at the given scale factor.
pub(crate) fn to_winit(pointer: Pointer, scale_factor: f32) -> winit::event::WindowEvent {
    let device_id = winit::event::DeviceId::dummy();
    match pointer {
        Pointer::Moved { x, y } => winit::event::WindowEvent::CursorMoved {
            device_id,
            position: winit::dpi::PhysicalPosition::new(
                (x * scale_factor) as f64,
                (y * scale_factor) as f64,
            ),
        },
        Pointer::Pressed | Pointer::Released => winit::event::WindowEvent::MouseInput {
            device_id,
            state: if pointer == Pointer::Pressed {
                winit::event::ElementState::Pressed
            } else {
                winit::event::ElementState::Released
            },
            button: winit::event::MouseButton::Left,
        },
        Pointer::Left => winit::event::WindowEvent::CursorLeft { device_id },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_scale_to_physical_pixels() {
        match to_winit(Pointer::Moved { x: 10.0, y: 20.5 }, 1.5) {
            winit::event::WindowEvent::CursorMoved { position, .. } => {
                assert_eq!((position.x, position.y), (15.0, 30.75));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn queue_drains_in_order() {
        push([Pointer::Pressed, Pointer::Released]);
        assert_eq!(drain(), [Pointer::Pressed, Pointer::Released]);
        assert!(drain().is_empty());
    }
}
