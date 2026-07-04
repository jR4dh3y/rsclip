use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use gio::prelude::*;
use gtk4 as gtk;

use crate::cli::UiCommand;

const APP_ID: &str = "io.github.radhey.rsclip";

pub(crate) fn run() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match crate::cli::parse_args(&args)? {
        UiCommand::List(args) => return crate::cli::cmd_list(&args),
        UiCommand::Help => {
            crate::cli::print_help();
            return Ok(());
        }
        UiCommand::Show | UiCommand::Toggle | UiCommand::Preload | UiCommand::QuitUi => {}
    }

    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        anyhow::bail!("rsclip overlay requires Wayland");
    }

    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    let runtime = Rc::new(RefCell::new(None));

    {
        let runtime = Rc::clone(&runtime);
        app.connect_command_line(move |app, command_line| {
            let args = command_line
                .arguments()
                .into_iter()
                .skip(1)
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>();

            match handle_command(app, &runtime, &args) {
                Ok(()) => gtk::glib::ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("rsclip: {err:#}");
                    gtk::glib::ExitCode::FAILURE
                }
            }
        });
    }

    let status = app.run();
    if status == gtk::glib::ExitCode::SUCCESS {
        Ok(())
    } else {
        anyhow::bail!("rsclip exited with status {}", status.get())
    }
}

fn handle_command(
    app: &gtk::Application,
    runtime: &Rc<RefCell<Option<crate::window::UiRuntime>>>,
    args: &[String],
) -> Result<()> {
    match crate::cli::parse_args(args)? {
        UiCommand::Show => {
            ensure_runtime(app, runtime)?;
            if let Some(runtime) = runtime.borrow().as_ref() {
                runtime.show_reset()?;
            }
        }
        UiCommand::Toggle => {
            ensure_runtime(app, runtime)?;
            if let Some(runtime) = runtime.borrow().as_ref() {
                runtime.toggle()?;
            }
        }
        UiCommand::Preload => {
            ensure_runtime(app, runtime)?;
            if let Some(runtime) = runtime.borrow().as_ref() {
                runtime.preload()?;
            }
        }
        UiCommand::QuitUi => {
            app.quit();
        }
        UiCommand::List(args) => {
            crate::cli::cmd_list(&args)?;
        }
        UiCommand::Help => crate::cli::print_help(),
    }
    Ok(())
}

fn ensure_runtime(
    app: &gtk::Application,
    runtime: &Rc<RefCell<Option<crate::window::UiRuntime>>>,
) -> Result<()> {
    if runtime.borrow().is_none() {
        match crate::window::build_ui(app) {
            Ok(ui) => {
                *runtime.borrow_mut() = Some(ui);
            }
            Err(err) => {
                app.quit();
                return Err(err);
            }
        }
    }
    Ok(())
}
