mod app;
mod camera;
mod creature;
mod daynight;
mod input;
mod llm;
mod net;
mod player;
mod raycast;
mod remote_player;
mod save;
mod scripting;
mod ui;
mod voxel;
mod weather;

use std::sync::Arc;

use winit::event::{Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::WindowBuilder;

fn main() {
    env_logger::init();

    let event_loop = EventLoop::new().expect("failed to create event loop");
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("Voxel Project")
            .with_inner_size(winit::dpi::PhysicalSize::new(1280, 720))
            .build(&event_loop)
            .expect("failed to create window"),
    );

    let launch_config = net::parse_args();
    let mut state = pollster::block_on(app::App::new(window.clone(), launch_config));
    window.request_redraw();

    event_loop
        .run(move |event, elwt| {
            elwt.set_control_flow(ControlFlow::Poll);
            match event {
                Event::WindowEvent { window_id, event } if window_id == window.id() => {
                    state.window_event(&event);
                    match event {
                        WindowEvent::CloseRequested => {
                            state.save();
                            elwt.exit();
                        }
                        WindowEvent::Resized(new_size) => {
                            state.resize(new_size);
                        }
                        WindowEvent::RedrawRequested => {
                            state.update();
                            match state.render() {
                                Ok(_) => {}
                                Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                                    state.resize(window.inner_size());
                                }
                                Err(wgpu::SurfaceError::OutOfMemory) => {
                                    log::error!("Out of memory, exiting.");
                                    elwt.exit();
                                }
                                Err(e) => log::warn!("Surface error: {e:?}"),
                            }
                            window.request_redraw();
                        }
                        _ => {}
                    }
                }
                Event::DeviceEvent { event, .. } => {
                    state.device_event(&event);
                }
                _ => {}
            }
        })
        .expect("event loop error");
}
