//! Shared editors for feature-specific compression settings.
//!
//! Automatic compression and Discord use [`CompressionOptions`]; the clipboard
//! exposes resize settings only. Keeping the controls in one module makes the
//! three cards consistent without sharing any profile state.

use gpui_kit::component::button::Button;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::*;
use vrc_photon_core::model::{CompressionFormat, CompressionOptions, ResizeMode, ResizeSettings};
use vrc_photon_i18n::tr;

use crate::app::App;
use crate::app::compression::FeatureCompressionTarget;
use crate::ui::components::chip;
use crate::ui::theme::format_label;

#[derive(Debug, Clone, Copy)]
enum ResizeOwner {
    Clipboard,
    Feature(FeatureCompressionTarget),
}

impl App {
    fn apply_resize_edit(
        &mut self,
        owner: ResizeOwner,
        edit: impl FnOnce(&mut ResizeSettings),
        cx: &mut Context<Self>,
    ) {
        match owner {
            ResizeOwner::Clipboard => self.edit_clipboard_resize(edit, cx),
            ResizeOwner::Feature(target) => {
                self.edit_feature_compression(target, |options| edit(&mut options.resize), cx)
            }
        }
    }

    /// Format, lossless, quality, and resize controls for one feature.
    pub(crate) fn compression_options_editor(
        &self,
        id_prefix: &'static str,
        options: &CompressionOptions,
        target: FeatureCompressionTarget,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette;
        let formats = [
            CompressionFormat::Jpeg,
            CompressionFormat::Webp,
            CompressionFormat::Avif,
            CompressionFormat::JpegXl,
        ];

        v_flex()
            .gap(px(7.))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(4.))
                    .children(formats.into_iter().map(|format| {
                        let active = options.format == format;
                        let selected = format.clone();
                        chip(&p, &format_label(&format), active)
                            .id(SharedString::from(format!(
                                "{id_prefix}-format-{}",
                                format.as_str()
                            )))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.edit_feature_compression(
                                    target,
                                    |options| options.set_format(selected.clone()),
                                    cx,
                                );
                            }))
                            .into_any_element()
                    })),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(88.))
                            .text_size(px(11.5))
                            .text_color(rgb(p.muted))
                            .child(tr!("compression.lossless")),
                    )
                    .child(
                        Switch::new(SharedString::from(format!("{id_prefix}-lossless")))
                            .checked(options.lossless)
                            .disabled(!options.format.supports_lossless())
                            .on_click(cx.listener(move |this, checked, _, cx| {
                                this.edit_feature_compression(
                                    target,
                                    |options| options.lossless = *checked,
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .w(px(88.))
                            .text_size(px(11.5))
                            .text_color(rgb(p.muted))
                            .child(tr!("compression.quality")),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{id_prefix}-quality-minus")))
                            .label("−")
                            .small()
                            .disabled(options.lossless)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.edit_feature_compression(
                                    target,
                                    |options| {
                                        options.quality = options.quality.saturating_sub(1).max(1);
                                    },
                                    cx,
                                );
                            })),
                    )
                    .child(
                        div()
                            .w(px(32.))
                            .text_size(px(12.))
                            .font_semibold()
                            .text_color(rgb(p.text))
                            .child(options.quality.to_string()),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{id_prefix}-quality-plus")))
                            .label("＋")
                            .small()
                            .disabled(options.lossless)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.edit_feature_compression(
                                    target,
                                    |options| {
                                        options.quality =
                                            options.quality.saturating_add(1).min(100);
                                    },
                                    cx,
                                );
                            })),
                    ),
            )
            .child(self.resize_editor_controls(
                id_prefix,
                &options.resize,
                ResizeOwner::Feature(target),
                cx,
            ))
            .into_any_element()
    }

    /// Resize-only controls for the clipboard feature.
    pub(crate) fn resize_settings_editor(
        &self,
        id_prefix: &'static str,
        resize: &ResizeSettings,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.resize_editor_controls(id_prefix, resize, ResizeOwner::Clipboard, cx)
    }

    fn resize_editor_controls(
        &self,
        id_prefix: &'static str,
        resize: &ResizeSettings,
        owner: ResizeOwner,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette;
        let mut body = v_flex().gap(px(6.)).child(
            h_flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .w(px(88.))
                        .text_size(px(11.5))
                        .text_color(rgb(p.muted))
                        .child(tr!("compression.enable_resize")),
                )
                .child(
                    Switch::new(SharedString::from(format!("{id_prefix}-resize")))
                        .checked(resize.resize_enabled)
                        .on_click(cx.listener(move |this, checked, _, cx| {
                            this.apply_resize_edit(
                                owner,
                                |resize| resize.resize_enabled = *checked,
                                cx,
                            );
                        })),
                ),
        );

        if resize.resize_enabled {
            body = body
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(4.))
                        .child(
                            div()
                                .w(px(88.))
                                .flex_shrink_0()
                                .text_size(px(11.5))
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
                            .map(|(label, width, height)| {
                                let active = resize.width == width && resize.height == height;
                                div()
                                    .id(SharedString::from(format!("{id_prefix}-resize-{label}")))
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
                                    .text_color(if active { rgb(p.accent) } else { rgb(p.muted) })
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.apply_resize_edit(
                                            owner,
                                            |resize| {
                                                resize.resize_enabled = true;
                                                resize.width = width;
                                                resize.height = height;
                                            },
                                            cx,
                                        );
                                    }))
                                    .child(label)
                                    .into_any_element()
                            }),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.muted_2))
                                .child(format!("{} × {}", resize.width, resize.height)),
                        ),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(4.))
                        .child(
                            div()
                                .w(px(88.))
                                .flex_shrink_0()
                                .text_size(px(11.5))
                                .text_color(rgb(p.muted))
                                .child(tr!("compression.resize_mode")),
                        )
                        .children(
                            [
                                (ResizeMode::DownscaleOnly, tr!("compression.resize.shrink")),
                                (ResizeMode::Always, tr!("compression.resize.always")),
                            ]
                            .into_iter()
                            .map(|(mode, label)| {
                                let active = resize.resize_mode == mode;
                                div()
                                    .id(SharedString::from(format!(
                                        "{id_prefix}-resize-mode-{}",
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
                                    .text_color(if active { rgb(p.accent) } else { rgb(p.muted) })
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.apply_resize_edit(
                                            owner,
                                            |resize| resize.resize_mode = mode,
                                            cx,
                                        );
                                    }))
                                    .child(label)
                                    .into_any_element()
                            }),
                        ),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .w(px(88.))
                                .flex_shrink_0()
                                .text_size(px(11.5))
                                .text_color(rgb(p.muted))
                                .child(tr!("compression.keep_aspect")),
                        )
                        .child(
                            Switch::new(SharedString::from(format!("{id_prefix}-aspect")))
                                .checked(resize.maintain_aspect)
                                .on_click(cx.listener(move |this, checked, _, cx| {
                                    this.apply_resize_edit(
                                        owner,
                                        |resize| resize.maintain_aspect = *checked,
                                        cx,
                                    );
                                })),
                        ),
                );
        }

        body.into_any_element()
    }
}
