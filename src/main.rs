mod app;
mod camera;
mod creature;
mod daynight;
mod input;
mod llm;
mod menu;
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

use menu::{MenuAction, MenuApp};
use net::LaunchConfig;

enum Stage {
    Menu(MenuApp),
    Game(app::App),
    /// Momentary placeholder used only while swapping `Menu` for `Game` (or
    /// back, on a failed connect) within a single event-loop iteration --
    /// never actually observed between iterations. Needed because dropping
    /// the old stage's wgpu device/surface has to happen *before* the new
    /// stage creates its own for the same window, or wgpu fatally errors
    /// with "device lost" from the two fighting over one surface.
    Transitioning,
}

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

    // `--connect` on the command line bypasses the menu entirely (used for
    // scripted/automated testing); any other launch starts at the main menu
    // so a normal double-click always lets the player choose new/load/join.
    let mut stage = if let Some(addr) = launch_config.connect {
        match pollster::block_on(app::App::new(window.clone(), launch_config)) {
            Ok(app) => Stage::Game(app),
            Err(e) => {
                eprintln!("Failed to connect to {addr}: {e}");
                std::process::exit(1);
            }
        }
    } else {
        Stage::Menu(pollster::block_on(MenuApp::new(window.clone(), launch_config)))
    };
    window.request_redraw();

    event_loop
        .run(move |event, elwt| {
            elwt.set_control_flow(ControlFlow::Poll);
            match event {
                Event::WindowEvent { window_id, event } if window_id == window.id() => {
                    match &mut stage {
                        Stage::Menu(menu) => {
                            menu.window_event(&event);
                            match event {
                                WindowEvent::CloseRequested => elwt.exit(),
                                WindowEvent::Resized(new_size) => menu.resize(new_size),
                                WindowEvent::RedrawRequested => {
                                    let action = menu.render();
                                    if let Some(action) = action {
                                        if matches!(action, MenuAction::Quit) {
                                            elwt.exit();
                                        } else {
                                            let port = menu.port;
                                            let llm_url = menu.llm_url.clone();
                                            let cfg = match action {
                                                MenuAction::NewWorld => LaunchConfig {
                                                    connect: None,
                                                    port,
                                                    llm_url: llm_url.clone(),
                                                    fresh: true,
                                                },
                                                MenuAction::LoadWorld => LaunchConfig {
                                                    connect: None,
                                                    port,
                                                    llm_url: llm_url.clone(),
                                                    fresh: false,
                                                },
                                                MenuAction::Join(addr) => LaunchConfig {
                                                    connect: Some(addr),
                                                    port,
                                                    llm_url: llm_url.clone(),
                                                    fresh: false,
                                                },
                                                MenuAction::Quit => unreachable!(),
                                            };
                                            // Drop the menu's device/surface
                                            // before the game creates its
                                            // own for this same window.
                                            stage = Stage::Transitioning;
                                            match pollster::block_on(app::App::new(
                                                window.clone(),
                                                cfg,
                                            )) {
                                                Ok(app) => stage = Stage::Game(app),
                                                Err(e) => {
                                                    let fallback = LaunchConfig {
                                                        connect: None,
                                                        port,
                                                        llm_url,
                                                        fresh: false,
                                                    };
                                                    let mut menu = pollster::block_on(
                                                        MenuApp::new(window.clone(), fallback),
                                                    );
                                                    menu.report_join_error(e);
                                                    stage = Stage::Menu(menu);
                                                }
                                            }
                                        }
                                    }
                                    window.request_redraw();
                                }
                                _ => {}
                            }
                        }
                        Stage::Game(state) => {
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
                        Stage::Transitioning => {}
                    }
                }
                Event::DeviceEvent { event, .. } => {
                    if let Stage::Game(state) = &mut stage {
                        state.device_event(&event);
                    }
                }
                _ => {}
            }
        })
        .expect("event loop error");
}
