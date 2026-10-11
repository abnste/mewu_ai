// SPDX-License-Identifier: MPL-2.0
//! GTK3 local file chooser, explicitly parented to the original host window.
//! No portal service is required. The selected filter determines the encoder;
//! the final suffix only validates that explicit choice.
use super::*;
use gtk::prelude::*;

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
        chooser.set_current_name(&name);
        let mut filters = Vec::new();
        for format in spec.formats {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(format.label));
            for extension in format.extensions {
                filter.add_pattern(&format!("*.{extension}"));
            }
            // GTK's transfer-full wrapper consumes its owned reference. Keep
            // our own clone for the explicit selected-format identity checks.
            chooser.add_filter(filter.clone());
            filters.push(filter);
        }
        chooser.set_filter(&filters[spec.default_index as usize - 1]);
        let old_index = std::cell::Cell::new(spec.default_index as usize - 1);
        let format_filters = filters.clone();
        chooser.connect_filter_notify(move |chooser| {
            let Some(index) = chooser
                .filter()
                .and_then(|f| format_filters.iter().position(|expected| *expected == f))
            else {
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
                        chooser.set_current_name(&format!(
                            "{base}.{}",
                            spec.formats[index].extensions[0]
                        ));
                    }
                }
            }
        });
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
                let index = chooser
                    .filter()
                    .and_then(|f| filters.iter().position(|expected| *expected == f))
                    .ok_or("保存格式不可用")?;
                let format = spec
                    .destination(&path, index as u32 + 1)
                    .map_err(|e| e.to_string())?;
                Ok(SaveDialogOutcome::Selected { path, format })
            }),
        ))
    })
    .map_err(SaveDialogError::Rejected)?;
    Ok(result.unwrap_or(SaveDialogOutcome::RequestCanceled))
}
