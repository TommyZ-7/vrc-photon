//! Compression settings screen: profile cards and the profile editor.
//!
//! Profile and manual-compression state lives in `app::compression`; screen
//! dispatch lives in `ui::shell`.

use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vrc_photon_core::model::{
    CompressionFormat, CompressionJob, CompressionProfile, MAX_JOB_RETRIES, MAX_PROFILE_NAME_CHARS,
    QueueStatus, ResizeMode,
};
use vrc_photon_i18n::{tr, tr_args};

use crate::app::App;
use crate::app::file_picker::{self, DialogOptions};
use crate::ui::components::{card_header, glass_panel};
use crate::ui::theme::*;

fn section_divider(palette: &Palette) -> AnyElement {
    let p = palette;
    div()
        .h(px(1.))
        .w_full()
        .bg(rgba(with_alpha(p.text, 0x0e)))
        .into_any_element()
}

/// Whether a failed job can return to the queue without exceeding the cap.
#[allow(dead_code)] // Used by the notification failure rows (Task 10).
pub(crate) fn job_retry_available(job: &CompressionJob) -> bool {
    job.status == QueueStatus::Failed && job.retry_count < MAX_JOB_RETRIES
}

/// The failure reason shown on a failed job's row. A job that failed
/// without a recorded reason still gets a line, so the row never reads
/// as if nothing went wrong.
#[allow(dead_code)] // Used by the notification failure rows (Task 10).
fn job_error_label(job: &CompressionJob) -> String {
    job.error
        .as_deref()
        .map(str::trim)
        .filter(|error| !error.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| tr!("compression.job.unknown_error"))
}

/// Retry usage for a failed job, or the cap explanation once exhausted.
#[allow(dead_code)] // Used by the notification failure rows (Task 10).
fn job_retry_label(job: &CompressionJob) -> String {
    if job_retry_available(job) {
        tr_args!(
            "compression.retry_label",
            count = job.retry_count,
            max = MAX_JOB_RETRIES
        )
    } else {
        tr_args!("compression.retry_limit", max = MAX_JOB_RETRIES)
    }
}

impl App {
    pub(crate) fn compression_screen(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = self.palette;
        let profile = self
            .profiles
            .get(self.profile_index)
            .or_else(|| self.profiles.first())
            .cloned();
        h_flex()
            .flex_1()
            .min_h_0()
            .gap(px(CARD_GAP))
            .items_stretch()
            .child(self.profile_list_card(cx))
            .child(match profile {
                Some(profile) => self.profile_editor(&profile, window, cx).into_any_element(),
                None => glass_panel(&self.palette)
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .rounded(px(17.))
                    .text_size(px(12.))
                    .text_color(rgb(p.muted_2))
                    .child(tr!("compression.no_profiles"))
                    .into_any_element(),
            })
    }

    /// Left card: profile list header with the add action, then the selectable
    /// profile cards in a scroll body.
    fn profile_list_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let profile_count = self.profiles.len();
        glass_panel(&self.palette)
            .flex()
            .flex_col()
            .w(px(COMPRESSION_PROFILE_WIDTH))
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .overflow_hidden()
            .rounded(px(17.))
            .child(self.profile_list_header(cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p(px(10.))
                    .gap(px(8.))
                    .children(self.profiles.iter().enumerate().map(|(index, profile)| {
                        let active = index == self.profile_index;
                        let title = profile.name.clone();
                        let meta = profile_summary(profile);
                        glass_panel(&self.palette)
                            .id(SharedString::from(format!("profile-{index}")))
                            .flex()
                            .flex_col()
                            .p(px(12.))
                            .gap(px(6.))
                            .rounded(px(12.))
                            .bg(rgb(if active { p.accent_soft } else { p.panel_inner }))
                            .border_color(rgb(if active { p.accent } else { p.border }))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.profile_index = index;
                                cx.notify();
                            }))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(12.))
                                            .font_semibold()
                                            .text_color(rgb(if active {
                                                p.accent
                                            } else {
                                                p.muted
                                            }))
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .id(SharedString::from(format!("profile-up-{index}")))
                                            .w(px(20.))
                                            .h(px(20.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(6.))
                                            .text_size(px(11.))
                                            .text_color(rgb(if index == 0 {
                                                p.muted_2
                                            } else {
                                                p.muted
                                            }))
                                            .when(index > 0, |this| {
                                                this.cursor_pointer()
                                                    .hover(|this| {
                                                        this.bg(rgb(p.panel_2))
                                                            .text_color(rgb(p.text))
                                                    })
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.move_profile(index, -1, cx);
                                                        cx.stop_propagation();
                                                    }))
                                            })
                                            .child("↑"),
                                    )
                                    .child(
                                        div()
                                            .id(SharedString::from(format!("profile-down-{index}")))
                                            .w(px(20.))
                                            .h(px(20.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(6.))
                                            .text_size(px(11.))
                                            .text_color(rgb(if index + 1 == profile_count {
                                                p.muted_2
                                            } else {
                                                p.muted
                                            }))
                                            .when(index + 1 < profile_count, |this| {
                                                this.cursor_pointer()
                                                    .hover(|this| {
                                                        this.bg(rgb(p.panel_2))
                                                            .text_color(rgb(p.text))
                                                    })
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.move_profile(index, 1, cx);
                                                        cx.stop_propagation();
                                                    }))
                                            })
                                            .child("↓"),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(if active { p.text } else { p.muted_2 }))
                                    .child(meta),
                            )
                    })),
            )
            .into_any_element()
    }

    /// Header bar for the profile column. Rehomed from the deleted page
    /// header; the add action keeps its original save/reload path.
    fn profile_list_header(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        card_header(&p)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.))
                    .font_semibold()
                    .text_color(rgb(p.text))
                    .child(tr!("compression.profiles")),
            )
            .child(
                Button::new("add-profile")
                    .label(tr!("compression.add"))
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        let profile = CompressionProfile {
                            name: tr_args!(
                                "compression.new_profile",
                                index = this.profiles.len() + 1
                            ),
                            ..CompressionProfile::default()
                        };
                        if let Err(error) = this.db.save_profile(&profile) {
                            tracing::warn!(%error, "cannot add compression profile");
                            this.toast_error(tr_args!("compression.add_error", error = error), cx);
                        }
                        this.profiles = this.db.list_profiles().unwrap_or_else(|error| {
                            tracing::warn!(%error, "cannot reload compression profiles");
                            Vec::new()
                        });
                        this.profile_index = this
                            .profiles
                            .iter()
                            .position(|item| item.id == profile.id)
                            .unwrap_or(0);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn profile_editor(
        &mut self,
        profile: &CompressionProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if self.profile_name_editing_id.as_deref() != Some(profile.id.as_str()) {
            let name = profile.name.clone();
            self.profile_name_input.update(cx, |input, cx| {
                input.set_value(name, window, cx);
            });
            self.profile_name_editing_id = Some(profile.id.clone());
            self.profile_name_reset_pending = false;
            self.profile_name_error = None;
        } else if self.profile_name_reset_pending {
            // Enter/blur normalizes trims and rejected values. Avoid rewriting
            // an unchanged field so its undo history and caret stay intact.
            if self.profile_name_input.read(cx).value().as_ref() != profile.name.as_str() {
                let name = profile.name.clone();
                self.profile_name_input.update(cx, |input, cx| {
                    input.set_value(name, window, cx);
                });
            }
            self.profile_name_reset_pending = false;
            self.profile_name_error = None;
        }
        let name_error = self.profile_name_error;
        let name_char_count = self
            .profile_name_input
            .read(cx)
            .value()
            .trim()
            .chars()
            .count();
        let p = self.palette;
        let profile_id = profile.id.clone();
        let suffix_value = self.profile_suffix_input.read(cx).value();
        if suffix_value.as_ref() != profile.suffix.as_str() {
            self.profile_suffix_input.update(cx, |input, cx| {
                input.set_value(profile.suffix.clone(), window, cx);
            });
        }
        let output_value = self.profile_output_input.read(cx).value();
        let output_current = profile.output_dir.clone().unwrap_or_default();
        if output_value.as_ref() != output_current.as_str() {
            self.profile_output_input.update(cx, |input, cx| {
                input.set_value(output_current, window, cx);
            });
        }
        let output_hint = match profile.output_dir.as_deref() {
            None => tr!("compression.output.empty"),
            Some(dir) if Path::new(dir).is_absolute() => tr!("compression.output.absolute"),
            Some(_) => tr!("compression.output.relative"),
        };
        glass_panel(&self.palette)
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .overflow_hidden()
            .rounded(px(17.))
            .child(
                card_header(&p)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(p.text))
                            .child(profile.name.clone()),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("duplicate-profile")
                            .label(tr!("compression.duplicate"))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| this.duplicate_profile(cx))),
                    )
                    .child(
                        Button::new("delete-profile")
                            .label(tr!("compression.delete"))
                            .small()
                            .danger()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.request_delete_profile(&profile_id, window, cx);
                            })),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p(px(14.))
                    .gap(px(14.))
                    .child(self.section_label(tr!("compression.section.basic")))
                    .child(
                        v_flex()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.muted_2))
                                    .child(tr!("compression.profile_name")),
                            )
                            .child(Input::new(&self.profile_name_input).small())
                            .child(
                                div()
                                    .text_size(px(9.5))
                                    .text_color(rgb(if name_error.is_some() {
                                        p.red
                                    } else {
                                        p.muted_2
                                    }))
                                    .child(match name_error {
                                        Some(error) => error.message(),
                                        None => tr_args!(
                                            "compression.name_hint",
                                            count = name_char_count,
                                            max = MAX_PROFILE_NAME_CHARS
                                        ),
                                    }),
                            ),
                    )
                    .child(section_divider(&p))
                    .child(self.section_label(tr!("compression.section.format")))
                    .child(
                        h_flex().gap(px(4.)).children(
                            [
                                CompressionFormat::Jpeg,
                                CompressionFormat::Webp,
                                CompressionFormat::Avif,
                                CompressionFormat::JpegXl,
                            ]
                            .into_iter()
                            .map(|format| {
                                let active = profile.format == format;
                                let label = format_label(&format);
                                let selected_format = format.clone();
                                div()
                                    .id(SharedString::from(format!("format-{}", format.as_str())))
                                    .px(px(10.))
                                    .py(px(5.))
                                    .rounded(px(8.))
                                    .bg(if active {
                                        rgb(p.accent_soft)
                                    } else {
                                        rgba(0x00000000)
                                    })
                                    .text_size(px(12.))
                                    .font_medium()
                                    .text_color(if active { rgb(p.accent) } else { rgb(p.muted) })
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.edit_profile(
                                            |profile| {
                                                apply_profile_format(
                                                    profile,
                                                    selected_format.clone(),
                                                )
                                            },
                                            cx,
                                        );
                                    }))
                                    .child(label)
                            }),
                        ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(80.))
                                    .text_size(px(12.))
                                    .text_color(rgb(p.muted))
                                    .child(tr!("compression.lossless")),
                            )
                            .child(
                                Switch::new("profile-lossless")
                                    .checked(profile.lossless)
                                    .disabled(!lossless_supported(&profile.format))
                                    .on_click(cx.listener(move |this, checked, _, cx| {
                                        this.edit_profile(
                                            |profile| profile.lossless = *checked,
                                            cx,
                                        );
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(80.))
                                    .text_size(px(12.))
                                    .text_color(rgb(p.muted))
                                    .child(tr!("compression.quality")),
                            )
                            .child(
                                Button::new("quality-minus")
                                    .label("−")
                                    .small()
                                    .disabled(profile.lossless)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.edit_profile(
                                            |profile| {
                                                profile.quality =
                                                    profile.quality.saturating_sub(1).max(1);
                                            },
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                div()
                                    .w(px(36.))
                                    .text_size(px(12.))
                                    .font_semibold()
                                    .text_color(rgb(p.text))
                                    .child(profile.quality.to_string()),
                            )
                            .child(
                                Button::new("quality-plus")
                                    .label("＋")
                                    .small()
                                    .disabled(profile.lossless)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.edit_profile(
                                            |profile| {
                                                profile.quality =
                                                    profile.quality.saturating_add(1).min(100);
                                            },
                                            cx,
                                        );
                                    })),
                            )
                            .when(!profile.lossless, |this| {
                                this.child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(p.muted_2))
                                        .child(tr!("compression.adjust_quality")),
                                )
                            }),
                    )
                    .child(section_divider(&p))
                    .child(self.section_label(tr!("compression.section.resize")))
                    .child(
                        v_flex()
                            .gap(px(8.))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(rgb(p.muted))
                                            .child(tr!("compression.enable_resize")),
                                    )
                                    .child(
                                        Switch::new("profile-resize")
                                            .checked(profile.resize_enabled)
                                            .on_click(cx.listener(|this, checked, _, cx| {
                                                this.edit_profile(
                                                    |profile| profile.resize_enabled = *checked,
                                                    cx,
                                                );
                                            })),
                                    ),
                            )
                            .when(profile.resize_enabled, |this| {
                                this.child(
                                    h_flex()
                                        .items_center()
                                        .gap_1()
                                        .child(
                                            div()
                                                .w(px(96.))
                                                .flex_shrink_0()
                                                .text_size(px(12.))
                                                .text_color(rgb(p.muted))
                                                .child(tr!("compression.resolution")),
                                        )
                                        .children(
                                            [
                                                ("HD", 1280u32, 720u32),
                                                ("FHD", 1920, 1080),
                                                ("2K", 2560, 1440),
                                                ("4K", 3840, 2160),
                                            ]
                                            .into_iter()
                                            .map(
                                                |(label, width, height)| {
                                                    let active = profile.width == width
                                                        && profile.height == height;
                                                    div()
                                                        .id(SharedString::from(format!(
                                                            "resize-{label}"
                                                        )))
                                                        .px(px(8.))
                                                        .py(px(4.))
                                                        .rounded(px(8.))
                                                        .bg(if active {
                                                            rgb(p.accent_soft)
                                                        } else {
                                                            rgba(0x00000000)
                                                        })
                                                        .text_size(px(10.))
                                                        .font_medium()
                                                        .text_color(if active {
                                                            rgb(p.accent)
                                                        } else {
                                                            rgb(p.muted)
                                                        })
                                                        .cursor_pointer()
                                                        .on_click(cx.listener(
                                                            move |this, _, _, cx| {
                                                                this.edit_profile(
                                                                    |profile| {
                                                                        profile.resize_enabled =
                                                                            true;
                                                                        profile.width = width;
                                                                        profile.height = height;
                                                                    },
                                                                    cx,
                                                                );
                                                            },
                                                        ))
                                                        .child(label)
                                                },
                                            ),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(10.))
                                                .text_color(rgb(p.muted_2))
                                                .child(format!(
                                                    "{} × {}",
                                                    profile.width, profile.height
                                                )),
                                        ),
                                )
                            })
                            .when(profile.resize_enabled, |this| {
                                this.child(
                                    v_flex()
                                        .gap(px(4.))
                                        .child(
                                            h_flex()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .w(px(96.))
                                                        .flex_shrink_0()
                                                        .text_size(px(12.))
                                                        .text_color(rgb(p.muted))
                                                        .child(tr!("compression.resize_mode")),
                                                )
                                                .child(
                                                    h_flex().gap(px(4.)).children(
                                                        [
                                                            (
                                                                ResizeMode::DownscaleOnly,
                                                                tr!("compression.resize.shrink"),
                                                            ),
                                                            (
                                                                ResizeMode::Always,
                                                                tr!("compression.resize.always"),
                                                            ),
                                                        ]
                                                        .into_iter()
                                                        .map(|(mode, label)| {
                                                            let active =
                                                                profile.resize_mode == mode;
                                                            div()
                                                                .id(SharedString::from(format!(
                                                                    "resize-mode-{}",
                                                                    mode.as_str()
                                                                )))
                                                                .px(px(8.))
                                                                .py(px(4.))
                                                                .rounded(px(8.))
                                                                .bg(if active {
                                                                    rgb(p.accent_soft)
                                                                } else {
                                                                    rgba(0x00000000)
                                                                })
                                                                .text_size(px(10.))
                                                                .font_medium()
                                                                .text_color(if active {
                                                                    rgb(p.accent)
                                                                } else {
                                                                    rgb(p.muted)
                                                                })
                                                                .cursor_pointer()
                                                                .on_click(cx.listener(
                                                                    move |this, _, _, cx| {
                                                                        this.edit_profile(
                                                                            |profile| {
                                                                                profile
                                                                                    .resize_mode =
                                                                                    mode;
                                                                            },
                                                                            cx,
                                                                        );
                                                                    },
                                                                ))
                                                                .child(label)
                                                        }),
                                                    ),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .pl(px(104.))
                                                .text_size(px(10.))
                                                .text_color(rgb(p.muted_2))
                                                .child(match profile.resize_mode {
                                                    ResizeMode::DownscaleOnly => {
                                                        tr!("compression.resize.shrink_hint")
                                                    }
                                                    ResizeMode::Always => {
                                                        tr!("compression.resize.always_hint")
                                                    }
                                                }),
                                        ),
                                )
                            })
                            .when(profile.resize_enabled, |this| {
                                this.child(
                                    h_flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .w(px(96.))
                                                .flex_shrink_0()
                                                .text_size(px(12.))
                                                .text_color(rgb(p.muted_2))
                                                .child(tr!("compression.keep_aspect")),
                                        )
                                        .child(
                                            Switch::new("profile-aspect")
                                                .checked(profile.maintain_aspect)
                                                .on_click(cx.listener(|this, checked, _, cx| {
                                                    this.edit_profile(
                                                        |profile| {
                                                            profile.maintain_aspect = *checked
                                                        },
                                                        cx,
                                                    );
                                                })),
                                        ),
                                )
                            }),
                    )
                    .child(section_divider(&p))
                    .child(self.section_label(tr!("compression.section.output")))
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap(px(12.))
                            .items_start()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w(px(300.))
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(rgb(p.muted_2))
                                            .child(tr!("compression.output_dir")),
                                    )
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap(px(6.))
                                            .child(
                                                Input::new(&self.profile_output_input)
                                                    .small()
                                                    .flex_1(),
                                            )
                                            .child(
                                                Button::new("profile-output-pick")
                                                    .label(tr!("compression.browse"))
                                                    .small()
                                                    .ghost()
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            let current = this
                                                                .profiles
                                                                .get(this.profile_index)
                                                                .and_then(|profile| {
                                                                    profile.output_dir.clone()
                                                                });
                                                            let options = DialogOptions {
                                                                directory: current
                                                                    .map(PathBuf::from),
                                                                ..DialogOptions::default()
                                                            };
                                                            let dialog =
                                                                options.dialog(Some(&*window));
                                                            cx.spawn_in(
                                                                window,
                                                                async move |this, cx| {
                                                                    let Some(dir) = file_picker::pick_folder(
                                                                        dialog,
                                                                    )
                                                                    .await
                                                                    else {
                                                                        return;
                                                                    };
                                                                    let path = dir
                                                                        .to_string_lossy()
                                                                        .to_string();
                                                                    let output_path =
                                                                        path.clone();
                                                                    cx.update(
                                                                        |window, cx| {
                                                                            this.update(
                                                                                cx,
                                                                                |this, cx| {
                                                                                    this.profile_output_input
                                                                                        .update(
                                                                                            cx,
                                                                                            |input, cx| {
                                                                                                input.set_value(
                                                                                                    path,
                                                                                                    window,
                                                                                                    cx,
                                                                                                )
                                                                                            },
                                                                                        );
                                                                                    this.edit_profile(
                                                                                        move |profile| {
                                                                                            profile.output_dir =
                                                                                                Some(output_path)
                                                                                        },
                                                                                        cx,
                                                                                    );
                                                                                },
                                                                            )
                                                                            .ok();
                                                                        },
                                                                    )
                                                                    .ok();
                                                                },
                                                            )
                                                            .detach();
                                                        },
                                                    )),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(9.5))
                                            .text_color(rgb(p.muted_2))
                                            .child(output_hint),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w(px(240.))
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(rgb(p.muted_2))
                                            .child(tr!("compression.suffix")),
                                    )
                                    .child(Input::new(&self.profile_suffix_input).small().flex_1())
                                    .child(
                                        div()
                                            .text_size(px(9.5))
                                            .text_color(rgb(p.muted_2))
                                            .child(tr!("compression.empty_ok")),
                                    ),
                            ),
                    )
                    .child(section_divider(&p))
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap(px(12.))
                    .p(px(12.))
                    .border_t_1()
                    .border_color(rgba(p.highlight))
                    .child(self.editor_stat(
                        tr!("compression.stats.total"),
                        self.stats.compressed_count.to_string(),
                    ))
                    .child(self.editor_stat(
                        tr!("compression.stats.photos"),
                        self.stats.photo_count.to_string(),
                    ))
                    .child(self.editor_stat(
                        tr!("compression.stats.compressed_total"),
                        format_bytes(self.stats.derivative_bytes),
                    )),
            )
    }

    /// Small stat block inside the editor card's footer.
    fn editor_stat(&self, label: impl Into<String>, value: String) -> impl IntoElement {
        let p = self.palette;
        let label: String = label.into();
        v_flex()
            .flex_1()
            .min_w_0()
            .gap(px(4.))
            .p(px(10.))
            .rounded(px(12.))
            .bg(rgb(p.panel_inner))
            .border_1()
            .border_color(rgba(with_alpha(p.text, 0x0f)))
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.muted_2))
                    .child(label),
            )
            .child(
                div()
                    .text_size(px(16.))
                    .font_semibold()
                    .text_color(rgb(p.text))
                    .truncate()
                    .child(value),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{job_error_label, job_retry_available, job_retry_label};
    use vrc_photon_core::model::{CompressionJob, CompressionOrigin, QueueStatus};

    fn test_job(status: QueueStatus, retry_count: i64) -> CompressionJob {
        CompressionJob {
            id: "job".to_string(),
            photo_id: 1,
            profile_id: Some("p".to_string()),
            origin: CompressionOrigin::Manual,
            status,
            progress: 0.0,
            error: Some("boom".to_string()),
            output_path: None,
            policy: "keep".to_string(),
            output_dir: None,
            created_at: "now".to_string(),
            started_at: None,
            finished_at: None,
            retry_count,
        }
    }

    #[test]
    fn job_retry_availability_stops_at_the_cap() {
        assert!(job_retry_available(&test_job(QueueStatus::Failed, 0)));
        assert!(job_retry_available(&test_job(QueueStatus::Failed, 2)));
        assert!(!job_retry_available(&test_job(QueueStatus::Failed, 3)));
        assert!(!job_retry_available(&test_job(QueueStatus::Completed, 0)));
        assert!(job_retry_label(&test_job(QueueStatus::Failed, 3)).contains("上限"));
    }

    #[test]
    fn job_error_label_falls_back_when_no_reason_was_recorded() {
        assert_eq!(job_error_label(&test_job(QueueStatus::Failed, 0)), "boom");
        let missing = CompressionJob {
            error: None,
            ..test_job(QueueStatus::Failed, 0)
        };
        assert_eq!(job_error_label(&missing), "原因不明");
        let blank = CompressionJob {
            error: Some("  ".to_string()),
            ..test_job(QueueStatus::Failed, 0)
        };
        assert_eq!(job_error_label(&blank), "原因不明");
    }
}
