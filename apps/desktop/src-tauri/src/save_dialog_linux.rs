// SPDX-License-Identifier: MPL-2.0
//! GTK3 local file chooser, explicitly parented to the original host window.
//! No portal service is required. The selected filter determines the encoder;
//! the final suffix only validates that explicit choice.
use super::*;
use gtk::prelude::*;
use std::sync::Mutex;

// Only a confirmed local destination is remembered, for this process lifetime.
// Never derive a save location from AppDir, the working directory or a renderer.
static LAST_DESTINATION_FOLDER: Mutex<Option<PathBuf>> = Mutex::new(None);

fn selected_filter_index(chooser: &gtk::FileChooserDialog, filters: &[gtk::FileFilter]) -> Option<usize> {
    chooser
        .filter()
        .and_then(|selected| filters.iter().position(|expected| *expected == selected))
}

fn configure_formats<F: Copy + Eq + 'static>(
    chooser: &gtk::FileChooserDialog,
    spec: DialogSpec<F>,
) -> Vec<gtk::FileFilter> {
    // The caller has validated this host-defined spec before creating GTK UI.
    let filters: Vec<_> = spec
        .formats
        .iter()
        .map(|format| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(format.label));
            for extension in format.extensions {
                filter.add_pattern(&format!("*.{extension}"));
            }
            filter
        })
        .collect();
    // GTK publicly permits a current filter without a user-selectable list.
    // Keeping that list empty avoids its bottom-edge popup entirely.
    chooser.set_filter(&filters[spec.default_index as usize - 1]);
    let old_index = std::cell::Cell::new(spec.default_index as usize - 1);
    let format_filters = filters.clone();
    chooser.connect_filter_notify(move |chooser| {
        let Some(index) = selected_filter_index(chooser, &format_filters) else {
            return;
        };
        let previous = old_index.replace(index);
        if previous == index {
            return;
        }
        if let Some(name) = chooser.current_name() {
            if let Some((base, extension)) = name.rsplit_once('.') {
                if spec.formats[previous]
                    .extensions
                    .iter()
                    .any(|value| extension.eq_ignore_ascii_case(value))
                {
                    chooser.set_current_name(&format!("{base}.{}", spec.formats[index].extensions[0]));
                }
            }
        }
    });
    if spec.formats.len() > 1 {
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let mut group: Option<gtk::RadioButton> = None;
        for (index, format) in spec.formats.iter().enumerate() {
            let button = match &group {
                Some(first) => gtk::RadioButton::with_label_from_widget(first, format.label),
                None => gtk::RadioButton::with_label(format.label),
            };
            if group.is_none() {
                group = Some(button.clone());
            }
            button.set_active(index == spec.default_index as usize - 1);
            let owner = chooser.downgrade();
            let selected = filters[index].clone();
            button.connect_toggled(move |button| {
                if button.is_active() {
                    if let Some(chooser) = owner.upgrade() {
                        chooser.set_filter(&selected);
                    }
                }
            });
            controls.pack_start(&button, false, false, 0);
        }
        controls.show_all();
        chooser.set_extra_widget(&controls);
    } else {
        // gtk-rs' method omits GTK's nullable form; use the public property.
        chooser.set_property("extra-widget", gtk::Widget::NONE);
    }
    filters
}

pub(crate) fn pick_destination<F: Copy + Eq + Send + Sync + 'static>(
    window: tauri::WebviewWindow,
    token: usize,
    spec: DialogSpec<F>,
    cancel: Arc<AtomicBool>,
    allowed: impl Fn() -> Result<(), String> + Send + 'static,
) -> Result<SaveDialogOutcome<F>, SaveDialogError> {
    spec.validate()?;
    let selected = spec.selected(spec.default_index)?;
    let name = format!(
        "{}.{}",
        default_filename(spec.basename, chrono::Local::now().naive_local()),
        selected.extensions[0]
    );
    let home = dirs::home_dir()
        .filter(|path| path.is_absolute() && path.is_dir())
        .ok_or_else(|| SaveDialogError::Rejected("用户目录不可用".into()))?;
    let remembered = LAST_DESTINATION_FOLDER
        .lock()
        .map_err(|_| SaveDialogError::Rejected("无法读取保存目录".into()))?
        .clone();
    // Directory checks run on the caller's blocking thread, outside GTK and
    // outside the memory lock. A removed destination falls back to the home.
    let folder = remembered
        .filter(|path| path.is_absolute() && path.is_dir())
        .unwrap_or_else(|| home.clone());
    let result = crate::gtk_dialog_linux::show(window, token, &cancel, &allowed, move |parent| {
        let chooser = gtk::FileChooserDialog::new(
            Some(crate::ui_language::text("保存图片", "Save image")),
            Some(parent),
            gtk::FileChooserAction::Save,
        );
        chooser.add_buttons(&[
            (
                crate::ui_language::text("取消", "Cancel"),
                gtk::ResponseType::Cancel,
            ),
            (
                crate::ui_language::text("保存", "Save"),
                gtk::ResponseType::Accept,
            ),
        ]);
        chooser.set_local_only(true);
        chooser.set_select_multiple(false);
        chooser.set_do_overwrite_confirmation(true);
        // This only requests GTK's folder; it does not prove asynchronous folder
        // loading has completed. Actual chooser navigation is verified in GUI.
        if !chooser.set_current_folder(&folder) && !chooser.set_current_folder(&home) {
            // No dialog session has been registered yet. Destroy this owned
            // GTK top-level explicitly before returning configuration failure.
            unsafe { chooser.destroy() };
            return Err("无法打开保存目录".into());
        }
        chooser.set_current_name(&name);
        let filters = configure_formats(&chooser, spec);
        Ok((
            chooser.upcast::<gtk::Dialog>(),
            Box::new(move |dialog: &gtk::Dialog, response| {
                if response != gtk::ResponseType::Accept {
                    return Ok(SaveDialogOutcome::UserCanceled);
                }
                let chooser = dialog
                    .downcast_ref::<gtk::FileChooserDialog>()
                    .ok_or("保存窗口类型不可用")?;
                let path = chooser.filename().ok_or("保存位置无效")?;
                let index = selected_filter_index(chooser, &filters).ok_or("保存格式不可用")?;
                let format = spec
                    .destination(&path, index as u32 + 1)
                    .map_err(|e| e.to_string())?;
                Ok(SaveDialogOutcome::Selected { path, format })
            }),
        ))
    })
    .map_err(SaveDialogError::Rejected)?;
    let result = result.unwrap_or(SaveDialogOutcome::RequestCanceled);
    if let SaveDialogOutcome::Selected { path, .. } = &result {
        // show() has acknowledged GTK destruction and checked the source and
        // cancellation. Recheck before changing the process-local preference.
        if cancel.load(Ordering::Acquire) {
            return Ok(SaveDialogOutcome::RequestCanceled);
        }
        allowed().map_err(SaveDialogError::Rejected)?;
        let folder = path.parent().ok_or(SaveDialogError::InvalidPath)?;
        *LAST_DESTINATION_FOLDER
            .lock()
            .map_err(|_| SaveDialogError::Rejected("无法记住保存目录".into()))? =
            Some(folder.to_path_buf());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Format {
        Mp4,
        Mp3,
        Gif,
    }
    static FORMATS: [FormatSpec<Format>; 3] = [
        FormatSpec {
            format: Format::Mp4,
            label: "MP4",
            extensions: &["mp4"],
        },
        FormatSpec {
            format: Format::Mp3,
            label: "MP3",
            extensions: &["mp3"],
        },
        FormatSpec {
            format: Format::Gif,
            label: "GIF",
            extensions: &["gif"],
        },
    ];

    // Run this exact GTK gate under Xvfb in the whole Docker image. It proves
    // public GTK semantics and format identity, not Wayland popup GUI acceptance.
    #[test]
    fn gtk_inline_formats_keep_owned_filter_identity_and_filename_contract() {
        gtk::init().expect("GTK gate requires its test display");
        let spec = DialogSpec {
            basename: "Recording",
            formats: &FORMATS,
            default_index: 2,
        };
        let chooser = gtk::FileChooserDialog::new(
            Some("test"),
            gtk::Window::NONE,
            gtk::FileChooserAction::Save,
        );
        chooser.set_current_name("draft.MP3");
        spec.validate().unwrap();
        let filters = configure_formats(&chooser, spec);
        assert!(chooser.list_filters().is_empty());
        assert_eq!(selected_filter_index(&chooser, &filters), Some(1));
        let controls = chooser
            .extra_widget()
            .unwrap()
            .downcast::<gtk::Box>()
            .unwrap();
        let buttons: Vec<_> = controls
            .children()
            .into_iter()
            .map(|child| child.downcast::<gtk::RadioButton>().unwrap())
            .collect();
        assert_eq!(buttons.len(), 3);
        assert_eq!(buttons.iter().filter(|button| button.is_active()).count(), 1);
        assert!(buttons[1].is_active());
        buttons[2].set_active(true);
        assert_eq!(selected_filter_index(&chooser, &filters), Some(2));
        assert_eq!(chooser.current_name().as_deref(), Some("draft.gif"));
        assert_eq!(
            spec.destination(Path::new("/tmp/draft.gif"), 3).unwrap(),
            Format::Gif
        );
        assert_eq!(buttons.iter().filter(|button| button.is_active()).count(), 1);
        chooser.set_current_name("manual.txt");
        buttons[0].set_active(true);
        assert_eq!(selected_filter_index(&chooser, &filters), Some(0));
        assert_eq!(chooser.current_name().as_deref(), Some("manual.txt"));
        assert!(matches!(
            spec.destination(Path::new("/tmp/manual.txt"), 1),
            Err(SaveDialogError::ExtensionConflict)
        ));
        let foreign = gtk::FileFilter::new();
        foreign.set_name(Some("MP4"));
        foreign.add_pattern("*.mp4");
        chooser.set_filter(&foreign);
        assert_eq!(selected_filter_index(&chooser, &filters), None);
        let owner = chooser.downgrade();
        unsafe { chooser.destroy() };
        drop(chooser);
        assert!(owner.upgrade().is_none());
        // Retained controls must not keep or resurrect their destroyed dialog.
        buttons[1].set_active(true);
        assert!(owner.upgrade().is_none());

        let single = DialogSpec {
            basename: "Recording",
            formats: &FORMATS[..1],
            default_index: 1,
        };
        let chooser = gtk::FileChooserDialog::new(
            Some("test"),
            gtk::Window::NONE,
            gtk::FileChooserAction::Save,
        );
        single.validate().unwrap();
        let filters = configure_formats(&chooser, single);
        assert!(chooser.list_filters().is_empty());
        assert!(chooser.extra_widget().is_none());
        assert_eq!(selected_filter_index(&chooser, &filters), Some(0));
        unsafe { chooser.destroy() };
    }
}
