#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
mod status_tray;

#[cfg(not(target_os = "linux"))]
mod status_tray_not_linux;

#[cfg(not(target_os = "macos"))]
mod tray_battery_icon_state;

#[cfg(feature = "eq-support")]
use hyper_headset::eq::session::EqSession;

#[cfg(not(feature = "eq-support"))]
fn warn_eq_unavailable_once(can_set_equalizer: bool) {
    if can_set_equalizer {
        static EQ_WARNING: std::sync::Once = std::sync::Once::new();
        EQ_WARNING.call_once(|| {
            eprintln!(
                "This headset supports EQ presets. Rebuild with --features eq-support to enable."
            )
        });
    }
}

/// A menu-bar app must outlive the Terminal window that started the raw binary.
#[cfg(target_os = "macos")]
fn ignore_terminal_hangup() {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = libc::SIG_IGN;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGHUP, &action, std::ptr::null_mut());
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    use clap::ArgAction;
    use std::sync::mpsc;

    use hyper_headset::devices::{DeviceEvent, DeviceProperties};
    use hyper_headset::VERBOSE;
    use winit::event_loop::{ControlFlow, EventLoop, EventLoopProxy};

    use crate::status_tray_not_linux::TrayApp;

    // Closing Terminal delivers SIGHUP to the foreground process group.
    #[cfg(target_os = "macos")]
    ignore_terminal_hangup();

    // The user event is the device state; `None` means no compatible device.
    // A regular, already-active app gets Quit in the Dock menu immediately.
    // Accessory apps only show that item after you click the Dock icon once.
    #[cfg(target_os = "macos")]
    let event_loop: EventLoop<Option<DeviceProperties>> = {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        let mut builder = EventLoop::with_user_event();
        builder
            .with_activation_policy(ActivationPolicy::Regular)
            .with_default_menu(true)
            .with_activate_ignoring_other_apps(true);
        builder.build().unwrap()
    };
    #[cfg(not(target_os = "macos"))]
    let event_loop: EventLoop<Option<DeviceProperties>> =
        EventLoop::with_user_event().build().unwrap();
    let proxy: EventLoopProxy<Option<DeviceProperties>> = event_loop.create_proxy();
    event_loop.set_control_flow(ControlFlow::Wait);

    let (tx, rx) = mpsc::channel::<DeviceEvent>();

    std::thread::spawn(move || {
        use std::time::Duration;

        use clap::{Arg, Command};
        use enigo::{Direction, Enigo, Key, Keyboard, Settings};

        use hyper_headset::devices::connect_compatible_device;

        // LaunchServices historically passed `-psn_…` to bundled apps.
        #[cfg(target_os = "macos")]
        let args = std::env::args().filter(|arg| !arg.starts_with("-psn_"));
        #[cfg(not(target_os = "macos"))]
        let args = std::env::args();

        // Battery is read on every tick. A full property sweep still happens every 30 ticks.
        const DEFAULT_REFRESH_SECS: &str = if cfg!(target_os = "macos") { "5" } else { "3" };

        let matches = Command::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .disable_version_flag(false)
        .author(env!("CARGO_PKG_AUTHORS"))
        .about("A tray application for monitoring HyperX headsets.")
        .arg(
            Arg::new("refresh-interval")
                .long("refresh-interval")
                .alias("refresh_interval")
                .required(false)
                .help("Set the refresh interval (in seconds)")
                .default_value(DEFAULT_REFRESH_SECS)
                .value_parser(clap::value_parser!(u64)),
        )
        .arg(
            Arg::new("press-mute-key")
                .long("press-mute-key")
                .alias("press_mute_key")
                .required(false)
                .help("The app will simulate pressing the microphone mute key whoever the headsets is muted or unmuted.")
                .default_value("true")
                .value_parser(clap::value_parser!(bool)),
        )
        .arg(Arg::new("verbose")
            .long("verbose")
            .short('v')
            .action(ArgAction::SetTrue)
            .required(false)
            .help("Use verbose output ")
        )
        .get_matches_from(args);

        VERBOSE.set(matches.get_flag("verbose")).unwrap();

        let press_mute_key = *matches.get_one::<bool>("press-mute-key").unwrap_or(&true);
        // Built on the first mute change. Creating it at launch asks macOS for
        // Accessibility ("control") before the headset has been used.
        let mut enigo: Option<Enigo> = None;
        let refresh_interval = *matches.get_one::<u64>("refresh-interval").unwrap_or(&3);
        let refresh_interval = Duration::from_secs(refresh_interval);

        #[cfg(feature = "eq-support")]
        let mut eq = EqSession::new();

        loop {
            // (Re-)connect to device
            let mut device = loop {
                match connect_compatible_device() {
                    Ok(d) => break d,
                    Err(e) => {
                        let _ = proxy.send_event(None);
                        eprintln!("Connecting failed with error: {e}")
                    }
                }
                std::thread::sleep(Duration::from_secs(1));
            };

            #[cfg(feature = "eq-support")]
            if let Some(ref mut eq) = eq {
                if let Some(dev) = device.hid_mut() {
                    eq.bind_device(&mut **dev);
                }
            }
            #[cfg(not(feature = "eq-support"))]
            warn_eq_unavailable_once(device.device_properties().can_set_equalizer);

            // Show battery before the slower full property sweep.
            let _ = device.passive_refresh_state();
            let _ = proxy.send_event(Some(device.device_properties()));

            // Run tick loop while connected
            let mut run_counter = 0;
            loop {
                let mute_state = device.device_properties().muted;
                match if run_counter % 30 == 0 {
                    device.active_refresh_state()
                } else {
                    device.passive_refresh_state()
                } {
                    Ok(()) => {
                        // Publish before the poll wait, or the menu bar stays empty
                        // for the whole refresh interval.
                        let _ = proxy.send_event(Some(device.device_properties()));
                    }
                    Err(error) => {
                        eprintln!("{error}");
                        let _ = proxy.send_event(Some(device.device_properties()));
                        break; // exit tick loop to retry connection in the outer loop
                    }
                };
                if mute_state.is_some() && mute_state != device.device_properties().muted && press_mute_key
                {
                    if enigo.is_none() {
                        match Enigo::new(&Settings::default()) {
                            Ok(instance) => enigo = Some(instance),
                            Err(e) => {
                                eprintln!("Virtual mute key failed to initialize: {e}");
                            }
                        }
                    }
                    if let Some(enigo) = enigo.as_mut() {
                        if let Err(e) = enigo.key(Key::F20, Direction::Click) {
                            eprintln!("Failed to press key on mute: {e}");
                        }
                    }
                }

                // Process tray device commands
                // with the default refresh_interval the state is only actively queried every 3min
                // querying the device too frequently can lead to instability
                let first = rx.recv_timeout(refresh_interval);
                // Spam-selecting EQ presets queues one event per click, but only the last
                // selection matters, so defer EQ commands and only apply the latest one.
                let mut pending_eq = None;
                for command in first.into_iter().chain(rx.try_iter()) {
                    if matches!(command, DeviceEvent::EqualizerPreset(_)) {
                        pending_eq = Some(command);
                        continue;
                    }
                    let _ = device.try_apply(command);
                    std::thread::sleep(hyper_headset::devices::RESPONSE_DELAY);
                    let _ = device.active_refresh_state();
                }
                if let Some(command) = pending_eq {
                    let _ = device.try_apply(command);
                    // EQ state is app-managed and never read back from any device, but the
                    // refresh also polls unrelated properties — only skip it for devices
                    // confirmed to gain nothing from it (see skip_refresh_after_eq_write).
                    if !device.skip_refresh_after_eq_write() {
                        std::thread::sleep(hyper_headset::devices::RESPONSE_DELAY);
                        let _ = device.active_refresh_state();
                    }
                }

                // Per-tick EQ session work: pick up watcher changes, sync
                // active preset on reconnect.
                #[cfg(feature = "eq-support")]
                if let Some(ref mut eq) = eq {
                    if let Some(dev) = device.hid_mut() {
                        eq.load_if_config_changed(&mut **dev);
                        eq.sync_if_reconnected(&mut **dev);
                    }
                }

                let _ = proxy.send_event(Some(device.device_properties()));
                run_counter += 1;
            }
        }
    });

    let tray_proxy = event_loop.create_proxy();
    event_loop
        .run_app(&mut TrayApp::new(tx, tray_proxy))
        .unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() {
    use clap::ArgAction;
    use clap::{Arg, Command};
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    use std::sync::mpsc;
    use std::time::Duration;

    use hyper_headset::devices::{connect_compatible_device, DeviceEvent};
    use status_tray::{StatusTray, TrayHandler};

    use hyper_headset::prompt_user_for_udev_rule;
    use hyper_headset::{act_as_askpass_handler, VERBOSE};

    if let Ok(name) = std::env::current_exe() {
        if let Some(name) = name.to_str() {
            if let Ok(askpass) = std::env::var("SUDO_ASKPASS") {
                if name == askpass {
                    act_as_askpass_handler();
                }
            }
        }
    }
    prompt_user_for_udev_rule();
    let matches = Command::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .disable_version_flag(false)
        .author(env!("CARGO_PKG_AUTHORS"))
        .about("A tray application for monitoring HyperX headsets.")
        .arg(
            Arg::new("refresh-interval")
                .long("refresh-interval")
                .alias("refresh_interval")
                .required(false)
                .help("Set the refresh interval (in seconds)")
                .default_value("3")
                .value_parser(clap::value_parser!(u64)),
        )
        .arg(
            Arg::new("press-mute-key")
                .long("press-mute-key")
                .alias("press_mute_key")
                .required(false)
                .help("The app will simulate pressing the microphone mute key whoever the headsets is muted or unmuted.")
                .default_value("true")
                .value_parser(clap::value_parser!(bool)),
        )
        .arg(Arg::new("verbose")
            .long("verbose")
            .short('v')
            .action(ArgAction::SetTrue)
            .required(false)
            .help("Use verbose output ")
        )
        .arg(Arg::new("monochrome_icons")
            .long("monochrome-icons")
            .action(ArgAction::SetTrue)
            .required(false)
            .help("Use the symbolic (monochrome) variants of the system tray icons")
        )
        .get_matches();

    let press_mute_key = *matches.get_one::<bool>("press-mute-key").unwrap_or(&true);
    let mut enigo = if press_mute_key {
        match Enigo::new(&Settings::default()) {
            Ok(enigo) => Some(enigo),
            Err(e) => {
                eprintln!("Virtual mute key failed to initialize: {e}");
                None
            }
        }
    } else {
        None
    };
    VERBOSE.set(matches.get_flag("verbose")).unwrap();
    let monochrome_icons = matches.get_flag("monochrome_icons");

    let refresh_interval = *matches.get_one::<u64>("refresh-interval").unwrap_or(&3);
    let refresh_interval = Duration::from_secs(refresh_interval);

    let (tx, rx) = mpsc::channel();
    let tray_handler = match TrayHandler::new(StatusTray::new(tx, monochrome_icons)).await {
        Ok(tray) => tray,
        Err(e) => {
            eprintln!("Failed to create the tray with error: {e:?}");
            return;
        }
    };

    #[cfg(feature = "eq-support")]
    let mut eq = EqSession::new();

    loop {
        // (Re-)connect to device
        let mut device = loop {
            match connect_compatible_device() {
                Ok(d) => break d,
                Err(e) => {
                    tray_handler.clear_state().await;
                    eprintln!("Connecting failed with error: {e}");
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        };

        #[cfg(feature = "eq-support")]
        if let Some(ref mut eq) = eq {
            if let Some(dev) = device.hid_mut() {
                eq.bind_device(&mut **dev);
            }
        }
        #[cfg(not(feature = "eq-support"))]
        warn_eq_unavailable_once(device.device_properties().can_set_equalizer);

        // Run tick loop while connected
        let mut run_counter = 0;
        loop {
            let mute_state = device.device_properties().muted;
            match if run_counter % 30 == 0 {
                device.active_refresh_state()
            } else {
                device.passive_refresh_state()
            } {
                Ok(()) => (),
                Err(error) => {
                    eprintln!("{error}");
                    tray_handler.update(&device.device_properties()).await;
                    break; // exit tick loop to retry connection in the outer loop
                }
            };
            if mute_state.is_some() && mute_state != device.device_properties().muted {
                if let Some(enigo) = &mut enigo {
                    if let Err(e) = enigo.key(Key::MicMute, Direction::Click) {
                        eprintln!("Failed to press key on mute: {e}");
                    }
                }
            }

            // Process tray device commands
            // with the default refresh_interval the state is only actively queried every 3min
            // querying the device too frequently can lead to instability
            let first = rx.recv_timeout(refresh_interval);
            // Spam-selecting EQ presets queues one event per click, but only the last
            // selection matters, so defer EQ commands and only apply the latest one.
            let mut pending_eq = None;
            for command in first.into_iter().chain(rx.try_iter()) {
                if matches!(command, DeviceEvent::EqualizerPreset(_)) {
                    pending_eq = Some(command);
                    continue;
                }
                let _ = device.try_apply(command);
                std::thread::sleep(hyper_headset::devices::RESPONSE_DELAY);
                let _ = device.active_refresh_state();
            }
            if let Some(command) = pending_eq {
                let _ = device.try_apply(command);
                // EQ state is app-managed and never read back from any device, but the
                // refresh also polls unrelated properties — only skip it for devices
                // confirmed to gain nothing from it (see skip_refresh_after_eq_write).
                if !device.skip_refresh_after_eq_write() {
                    std::thread::sleep(hyper_headset::devices::RESPONSE_DELAY);
                    let _ = device.active_refresh_state();
                }
            }

            // Per-tick EQ session work: pick up watcher changes, sync
            // active preset on reconnect.
            #[cfg(feature = "eq-support")]
            if let Some(ref mut eq) = eq {
                if let Some(dev) = device.hid_mut() {
                    eq.load_if_config_changed(&mut **dev);
                    eq.sync_if_reconnected(&mut **dev);
                }
            }

            tray_handler.update(&device.device_properties()).await;

            run_counter += 1;
        }
    }
}
