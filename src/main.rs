mod app;
mod audio;
mod camera;
mod creature;
mod crafting;
mod crafting_ui;
mod resource_ui;
mod daynight;
mod input;
mod equipment;
mod equipment_ui;
mod held_item;
mod llm;
mod llm_server;
mod menu;
mod model;
mod net;
mod transport;
#[cfg(feature = "steam")]
mod steam_transport;
mod player;
mod raycast;
mod block_target;
mod remote_player;
mod save;
mod settings;
mod ui_theme;
#[cfg(test)]
mod ui_preview_tests;
mod script_budget;
mod scripting;
mod ui;
mod voxel;
mod visibility;
mod weather;
mod wind;
mod world_api_gen;
mod world_api_validate;

use std::sync::Arc;

use winit::event::{ElementState, Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, WindowBuilder};

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

    #[cfg(feature = "steam")]
    {
        let settings = settings::Settings::load(std::path::Path::new("settings.json")).unwrap_or_default();
        if let Err(e) = steam_transport::initialize(settings.multiplayer.steam_app_id) {
            log::warn!("{e}"); // Direct mode remains usable without a signed-in Steam client.
        }
    }
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("Voxel Project")
            .with_inner_size(winit::dpi::PhysicalSize::new(1280, 720))
            .build(&event_loop)
            .expect("failed to create window"),
    );

    let launch_config = net::parse_args();

    // Checked/started on a background thread so a forgotten-after-reboot
    // llama-server doesn't block the window from opening -- rule generation
    // just stays unavailable until it finishes coming up.
    {
        let llm_url = launch_config.llm_url.clone();
        std::thread::spawn(move || llm_server::ensure_running(&llm_url));
    }

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
                    // Handled once here (not per-stage) so F11 works the
                    // same whether you're at the menu or in-game.
                    // `!repeat` avoids the OS auto-repeating a held key into
                    // rapid fullscreen/windowed thrashing.
                    if let WindowEvent::KeyboardInput { event: key_event, .. } = &event {
                        if key_event.state == ElementState::Pressed
                            && !key_event.repeat
                            && key_event.physical_key == PhysicalKey::Code(KeyCode::F11)
                        {
                            window.set_fullscreen(if window.fullscreen().is_some() {
                                None
                            } else {
                                Some(Fullscreen::Borderless(None))
                            });
                        }
                    }
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
                                                MenuAction::NewWorld { nickname } => LaunchConfig {
                                                    connect: None,
                                                    port,
                                                    llm_url: llm_url.clone(),
                                                    fresh: true,
                                                    nickname,
                                                },
                                                MenuAction::LoadWorld { nickname } => LaunchConfig {
                                                    connect: None,
                                                    port,
                                                    llm_url: llm_url.clone(),
                                                    fresh: false,
                                                    nickname,
                                                },
                                                MenuAction::Join { addr, nickname } => LaunchConfig {
                                                    connect: Some(addr),
                                                    port,
                                                    llm_url: llm_url.clone(),
                                                    fresh: false,
                                                    nickname,
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
                                                        // MenuApp doesn't read this --
                                                        // it re-prompts for a nickname
                                                        // before any launch.
                                                        nickname: String::new(),
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
                                    if state.wants_return_to_menu() {
                                        // Same save-then-drop-then-create
                                        // dance as CloseRequested, except we
                                        // land on a fresh menu instead of
                                        // exiting -- the old device/surface
                                        // must go before the menu creates
                                        // its own for this window.
                                        state.save();
                                        stage = Stage::Transitioning;
                                        let menu = pollster::block_on(MenuApp::new(
                                            window.clone(),
                                            LaunchConfig {
                                                connect: None,
                                                port: net::DEFAULT_PORT,
                                                llm_url: net::DEFAULT_LLM_URL.to_string(),
                                                fresh: false,
                                                // MenuApp doesn't read this --
                                                // it re-prompts for a nickname
                                                // before any launch.
                                                nickname: String::new(),
                                            },
                                        ));
                                        stage = Stage::Menu(menu);
                                        window.request_redraw();
                                        return;
                                    }
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
