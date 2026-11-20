//! App shell: sidebar navigation, global alerts, and the screen dispatch
//! wrapper.
//!
//! `App::render` stays in `app.rs`; this module owns the chrome around the
//! active screen and the grid helpers shared by the screens.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use vrc_photon_core::queue::{QueueCounts, QueueFeature, QueueProgress};
use vrc_photon_i18n::{tr, tr_args};
use vrc_photon_ipc::ScanPhase as IpcScanPhase;

use crate::app::ai::effective_ai_tagging_enabled;
use crate::app::lifecycle::ScanProgressView;
use crate::app::{App, Screen, SidebarStatusTab};
use crate::ui::components::status_progress_bar;
use crate::ui::queue::{queue_action_availability, queue_progress_percent, queue_status_breakdown};
use crate::ui::theme::*;

const NAV_ITEM_HEIGHT: f32 = 40.0;
const NAV_ITEM_GAP: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatusTone {
    Running,
    Waiting,
}

impl StatusTone {
    fn color(self, p: &Palette) -> u32 {
        match self {
            Self::Running => p.accent,
            Self::Waiting => p.amber,
        }
    }
}

impl App {
    pub(crate) fn sidebar(
        &mut self,
        window: &Window,
        content_height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette;
        let collapsed = self.sidebar_collapsed;
        let motion = self.sidebar_motion;
        let photo_count = format_count(self.library.stats.photo_count);
        let status_tabs = sidebar_status_tabs(
            self.settings.auto_compress_enabled,
            self.settings.discord_enabled,
            effective_ai_tagging_enabled(self.settings.ai_tagging_enabled, self.ai.installed()),
        );
        let status_card =
            (!collapsed && sidebar_status_visible(content_height) && !status_tabs.is_empty())
                .then(|| self.sidebar_status_card(&status_tabs, cx));

        let body = v_flex()
            .relative()
            .flex_shrink_0()
            .h_full()
            .py(px(16.))
            .px(px(12.))
            .gap(px(3.))
            .bg(rgb(p.panel_sidebar))
            .rounded(px(22.))
            .border_1()
            .border_color(rgba(p.line_soft))
            .overflow_hidden()
            .shadow(vec![
                BoxShadow::new(px(0.), px(18.), rgba(p.shadow).into()).blur_radius(px(44.)),
                BoxShadow::new(px(0.), px(1.), rgba(p.highlight).into())
                    .inset()
                    .blur_radius(px(0.)),
            ])
            .child(
                // The absolute highlight and the rows share this box. The
                // floating toggle sits outside the content edge, so no right
                // gutter is needed.
                v_flex()
                    .relative()
                    .w_full()
                    .gap(px(NAV_ITEM_GAP))
                    .child(self.nav_highlight(window, cx))
                    .child(self.nav_item(
                        window,
                        tr!("nav.photos"),
                        Screen::Photos,
                        IconName::GalleryVerticalEnd,
                        cx,
                    ))
                    .child(self.nav_item(
                        window,
                        tr!("nav.search"),
                        Screen::Search,
                        IconName::Search,
                        cx,
                    ))
                    .child(self.nav_item(
                        window,
                        tr!("nav.compression_short"),
                        Screen::Compression,
                        IconName::Minimize,
                        cx,
                    ))
                    .child(self.nav_item(
                        window,
                        tr!("nav.dashboard"),
                        Screen::Dashboard,
                        IconName::LayoutDashboard,
                        cx,
                    ))
                    .child(self.nav_item(
                        window,
                        tr!("nav.integrations"),
                        Screen::Integrations,
                        IconName::Bot,
                        cx,
                    ))
                    .child(self.nav_item(
                        window,
                        tr!("nav.settings"),
                        Screen::Settings,
                        IconName::Settings,
                        cx,
                    )),
            )
            .child(div().flex_1())
            .when_some(status_card, |this, card| this.child(card))
            .when(!collapsed, |this| {
                this.child(
                    v_flex()
                        .p(px(11.))
                        .gap(px(3.))
                        .rounded(px(13.))
                        .bg(rgb(p.panel))
                        .border_1()
                        .border_color(rgba(with_alpha(p.text, 0x0f)))
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.muted_2))
                                .child(tr!("sidebar.library")),
                        )
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_semibold()
                                .text_color(rgb(p.text))
                                .child(tr_args!("sidebar.photo_count", count = photo_count)),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.muted))
                                .child(format_bytes(self.library.stats.total_bytes)),
                        ),
                )
            })
            .when(collapsed, |this| this.child(self.sidebar_compact_stats()))
            .child(
                div()
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(if collapsed { 9. } else { 10. }))
                    .text_color(rgb(p.muted_2))
                    .child(if self.demo_assets.is_some() {
                        "DEMO DATA".to_string()
                    } else {
                        format!("v{}", env!("CARGO_PKG_VERSION"))
                    }),
            );

        body.with_animation(
            ("sidebar", motion.epoch()),
            Animation::new(motion.driver_duration())
                .with_easing(ease_motion)
                .with_max_fps(self.motion_fps(window, cx)),
            move |el, _| {
                let progress = motion.value();
                let from = sidebar_width(false);
                let to = sidebar_width(true);
                el.w(px(from + (to - from) * progress))
            },
        )
        .into_any_element()
    }

    pub(crate) fn rail_toggle(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let collapsed = self.sidebar_collapsed;
        let progress = self.sidebar_motion.value();
        let icon = if collapsed {
            IconName::ChevronRight
        } else {
            IconName::ChevronLeft
        };
        let tooltip = if collapsed {
            tr!("sidebar.toggle.expand")
        } else {
            tr!("sidebar.toggle.collapse")
        };
        let (from_left, top) = rail_toggle_origin(false);
        let (to_left, _) = rail_toggle_origin(true);
        let left = from_left + (to_left - from_left) * progress;

        div()
            .absolute()
            .top(px(top))
            .left(px(left))
            .child(
                Button::new("sidebar-toggle")
                    .ghost()
                    .icon(icon)
                    .tooltip(tooltip)
                    .size(px(24.))
                    .rounded_full()
                    .bg(rgb(p.panel_2))
                    .border_1()
                    .border_color(rgb(p.border_strong))
                    .text_color(rgb(p.muted))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .into_any_element()
    }

    fn sidebar_compact_stats(&self) -> AnyElement {
        let p = self.palette;
        v_flex()
            .p(px(6.))
            .gap(px(3.))
            .rounded(px(10.))
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgba(with_alpha(p.text, 0x0f)))
            .child(
                div()
                    .text_size(px(7.5))
                    .text_color(rgb(p.text))
                    .whitespace_nowrap()
                    .child(tr_args!(
                        "sidebar.photo_count_short",
                        count = format_count(self.library.stats.photo_count)
                    )),
            )
            .child(
                div()
                    .text_size(px(7.5))
                    .text_color(rgb(p.muted))
                    .whitespace_nowrap()
                    .truncate()
                    .child(format_bytes(self.library.stats.total_bytes)),
            )
            .into_any_element()
    }

    fn sidebar_status_card(&self, tabs: &[SidebarStatusTab], cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let Some(active_tab) = resolve_status_tab(self.sidebar_status_tab, tabs) else {
            return div().into_any_element();
        };
        let (counts, progress, paused) = self.status_tab_state(active_tab);

        let tab_row = h_flex()
            .gap(px(4.))
            .children(tabs.iter().copied().map(|tab| {
                let active = tab == active_tab;
                let (tab_counts, _, _) = self.status_tab_state(tab);
                h_flex()
                    .id(status_tab_element_id(tab))
                    .relative()
                    .flex_1()
                    .h(px(20.))
                    .justify_center()
                    .items_center()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(if active {
                        rgba(with_alpha(p.accent, 0x33))
                    } else {
                        rgba(0x00000000)
                    })
                    .text_size(px(10.))
                    .font_medium()
                    .cursor_pointer()
                    .when(active, |this| {
                        this.bg(rgb(p.accent_soft)).text_color(rgb(p.accent))
                    })
                    .when(!active, |this| {
                        this.text_color(rgb(p.muted_2))
                            .hover(|this| this.bg(rgb(p.panel_2)).text_color(rgb(p.text)))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sidebar_status_tab = tab;
                        cx.notify();
                    }))
                    .when_some(queue_attention(tab_counts), |this, tone| {
                        this.child(
                            div()
                                .absolute()
                                .top(px(3.))
                                .right(px(4.))
                                .size(px(4.))
                                .rounded_full()
                                .bg(rgb(tone.color(&p))),
                        )
                    })
                    .child(status_tab_label(tab))
            }));

        let body = match queue_progress_percent(counts, progress) {
            Some(percent) => self.status_progress_body(counts, percent),
            None => v_flex()
                .h(px(46.))
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(p.muted_2))
                        .child(tr!("sidebar.status.idle.headline")),
                )
                .into_any_element(),
        };

        let footer = if counts.waiting > 0 {
            let feature = status_tab_feature(active_tab);
            let start_enabled = queue_action_availability(!paused, counts.waiting).start_waiting;
            h_flex()
                .items_center()
                .h(px(24.))
                .gap(px(6.))
                .child(
                    Button::new("sidebar-status-start-waiting")
                        .label(tr!("sidebar.start_waiting"))
                        .small()
                        .primary()
                        .disabled(!start_enabled)
                        .tab_stop(start_enabled)
                        .when(start_enabled, |button| {
                            button.on_click(cx.listener(move |this, _, window, cx| {
                                this.release_waiting_queue(feature, window, cx)
                            }))
                        }),
                )
                .when(paused, |this| {
                    this.child(
                        div()
                            .text_size(px(9.5))
                            .text_color(rgb(p.amber))
                            .child(tr!("queue.paused")),
                    )
                })
        } else {
            h_flex().items_center().h(px(24.)).gap(px(6.)).child(
                Button::new("sidebar-status-open-queue")
                    .label(tr!("sidebar.open_queue"))
                    .small()
                    .ghost()
                    .on_click(
                        cx.listener(|this, _, _, cx| this.switch_screen(Screen::Dashboard, cx)),
                    ),
            )
        };

        v_flex()
            .flex_shrink_0()
            .p(px(9.))
            .gap(px(4.))
            .rounded(px(13.))
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgba(with_alpha(p.text, 0x0f)))
            .child(
                h_flex().items_center().h(px(12.)).child(
                    div()
                        .text_size(px(10.))
                        .font_semibold()
                        .text_color(rgb(p.muted_2))
                        .child(tr!("sidebar.progress")),
                ),
            )
            .child(tab_row)
            .child(body)
            .child(footer)
            .into_any_element()
    }

    fn status_progress_body(&self, counts: QueueCounts, percent: u8) -> AnyElement {
        let p = self.palette;
        v_flex()
            .h(px(46.))
            .justify_center()
            .gap(px(5.))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(7.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(status_progress_bar(&p, percent as f32 / 100.0)),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .font_semibold()
                            .text_color(rgb(p.text))
                            .child(format!("{percent}%")),
                    ),
            )
            .child(
                h_flex().items_center().gap(px(6.)).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .whitespace_nowrap()
                        .text_size(px(9.))
                        .text_color(rgb(p.muted_2))
                        .child(queue_status_breakdown(counts)),
                ),
            )
            .into_any_element()
    }

    fn status_tab_state(&self, tab: SidebarStatusTab) -> (QueueCounts, QueueProgress, bool) {
        match tab {
            SidebarStatusTab::AutoCompression => (
                self.compression_queue_counts,
                self.compression_queue_progress,
                self.settings.auto_compress_queue_paused,
            ),
            SidebarStatusTab::Discord => (
                self.discord_queue_counts,
                self.discord_queue_progress,
                self.settings.discord_queue_paused,
            ),
            SidebarStatusTab::AiTagging => (
                self.tagging_queue_counts,
                self.tagging_queue_progress,
                self.settings.ai_tagging_queue_paused,
            ),
        }
    }

    fn nav_highlight(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let p = self.palette;
        let nav_epoch = self.nav_epoch;
        let from = nav_item_top(nav_index(self.nav_previous));
        let to = nav_item_top(nav_index(self.screen));
        let vertical_duration = if nav_epoch == 0 {
            Duration::ZERO
        } else {
            MOTION_BASE
        };
        let vertical_anim = || {
            Animation::new(vertical_duration)
                .with_easing(ease_motion)
                .with_max_fps(self.motion_fps(window, cx))
        };

        let pill = div()
            .absolute()
            .left_0()
            .right_0()
            .h(px(NAV_ITEM_HEIGHT))
            .rounded(px(11.))
            .bg(rgb(p.accent_soft))
            .with_animation(("nav-pill", nav_epoch), vertical_anim(), move |el, t| {
                el.top(px(from + (to - from) * t))
            });

        let progress = self.sidebar_motion.value();
        let from_left = nav_bar_left(false);
        let to_left = nav_bar_left(true);
        let bar = div()
            .absolute()
            .left(px(from_left + (to_left - from_left) * progress))
            .top_0()
            .w(px(2.))
            .h_full()
            .child(
                div()
                    .absolute()
                    .top(px(from + 11.0))
                    .left_0()
                    .w(px(2.))
                    .h(px(18.))
                    .rounded_full()
                    .bg(rgb(p.accent))
                    .with_animation(("nav-bar-y", nav_epoch), vertical_anim(), move |el, t| {
                        el.top(px(from + 11.0 + (to - from) * t))
                    }),
            );

        div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(6.0 * NAV_ITEM_HEIGHT + 5.0 * NAV_ITEM_GAP))
            .child(pill)
            .child(bar)
            .into_any_element()
    }

    fn nav_item(
        &mut self,
        window: &Window,
        label: impl Into<SharedString>,
        screen: Screen,
        icon: IconName,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = label.into();
        let p = self.palette;
        let active = self.screen == screen;
        let was_active = self.nav_previous == screen;
        let hovered = self.hovered_nav == Some(screen);
        let reveal_progress = self.sidebar_motion.value();
        let nav_epoch = self.nav_epoch;
        let color_anim = || {
            Animation::new(MOTION_FAST)
                .with_easing(ease_motion)
                .with_max_fps(self.motion_fps(window, cx))
        };

        let icon_element = Icon::new(icon).with_size(px(17.)).with_animation(
            SharedString::from(format!("nav-icon-{label}-{nav_epoch}")),
            color_anim(),
            move |el, t| {
                if hovered && !active {
                    return el.text_color(rgb(p.text));
                }
                let from = if was_active { p.accent } else { p.muted };
                let to = if active { p.accent } else { p.muted };
                el.text_color(rgb(lerp_color(from, to, t)))
            },
        );

        let label_from = if was_active { p.text } else { p.muted };
        let label_to = if active || hovered { p.text } else { p.muted };
        let label_block = h_flex()
            .min_w_0()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .child(
                div()
                    .text_size(px(12.))
                    .child(label.clone())
                    .with_animation(
                        SharedString::from(format!("nav-label-{label}-{nav_epoch}")),
                        color_anim(),
                        move |el, t| el.text_color(rgb(lerp_color(label_from, label_to, t))),
                    ),
            )
            .w(px(sidebar_reveal_width(reveal_progress)))
            .opacity(1.0 - reveal_progress);

        let (pad, gap) = nav_item_insets(reveal_progress);
        let content = h_flex()
            .w_full()
            .items_center()
            .pl(px(pad))
            .gap(px(gap))
            .child(icon_element)
            .child(label_block);

        h_flex()
            .id(label)
            .relative()
            .h(px(NAV_ITEM_HEIGHT))
            .w_full()
            .rounded(px(11.))
            .cursor_pointer()
            .items_center()
            .when(!active, |this| {
                this.hover(|this| this.bg(rgb(p.panel_inner)).text_color(rgb(p.text)))
            })
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                // Hover listeners fire in reverse paint order. Let the entering
                // row win instead of letting the leaving row clear it.
                if *hovered {
                    if this.hovered_nav != Some(screen) {
                        this.hovered_nav = Some(screen);
                        cx.notify();
                    }
                } else if this.hovered_nav == Some(screen) {
                    this.hovered_nav = None;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.switch_screen(screen, cx);
            }))
            .child(content)
            .into_any_element()
    }

    pub(crate) fn nav_tooltip(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let p = self.palette;
        let Some(screen) = self.hovered_nav.filter(|_| self.sidebar_collapsed) else {
            // Keep the empty slot out of the root flex flow; an in-flow
            // zero-width child would still consume a 14px gap and shrink the
            // main column on every layout.
            return div().absolute().into_any_element();
        };
        let index = nav_index(screen);
        let (left, top) = nav_tooltip_origin(index);

        div()
            .absolute()
            .left(px(left - 4.0))
            .top(px(top))
            .px(px(8.))
            .py(px(5.))
            .rounded(px(7.))
            .bg(rgb(p.panel_card))
            .border_1()
            .border_color(rgb(p.border_strong))
            .text_size(px(10.))
            .text_color(rgb(p.text))
            .whitespace_nowrap()
            .child(screen.title())
            .with_animation(
                ("nav-tooltip", index),
                Animation::new(MOTION_FAST)
                    .with_easing(ease_motion)
                    .with_max_fps(self.motion_fps(window, cx)),
                move |el, t| el.opacity(t).left(px(left - 4.0 + 4.0 * t)),
            )
            .into_any_element()
    }

    pub(crate) fn content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let screen = match self.screen {
            Screen::Photos => self.photos_screen(window, cx).into_any_element(),
            Screen::Compression => self.compression_screen(window, cx).into_any_element(),
            Screen::Search => self.search_screen(window, cx).into_any_element(),
            Screen::Dashboard => self.dashboard_screen(cx).into_any_element(),
            Screen::Integrations => self.integrations_screen(window, cx).into_any_element(),
            Screen::Settings => self.settings_screen(cx).into_any_element(),
        };
        let motion = self.screen_motion;
        let enter = Animation::new(motion.driver_duration())
            .with_easing(ease_motion)
            .with_max_fps(self.motion_fps(window, cx));
        let direction = self.screen_direction;
        let screen = v_flex().flex_1().min_h_0().child(screen).with_animation(
            ("screen-enter", motion.epoch()),
            enter,
            move |el, _| {
                el.relative()
                    .left(px(screen_enter_offset(direction, motion.value())))
            },
        );
        v_flex().flex_1().min_h_0().child(screen).into_any_element()
    }
}

fn sidebar_status_tabs(auto: bool, discord: bool, ai: bool) -> Vec<SidebarStatusTab> {
    let mut tabs = Vec::with_capacity(3);
    if auto {
        tabs.push(SidebarStatusTab::AutoCompression);
    }
    if discord {
        tabs.push(SidebarStatusTab::Discord);
    }
    if ai {
        tabs.push(SidebarStatusTab::AiTagging);
    }
    tabs
}

fn resolve_status_tab(
    selected: SidebarStatusTab,
    visible: &[SidebarStatusTab],
) -> Option<SidebarStatusTab> {
    visible
        .contains(&selected)
        .then_some(selected)
        .or_else(|| visible.first().copied())
}

fn queue_attention(counts: QueueCounts) -> Option<StatusTone> {
    if counts.processing > 0 || counts.pending > 0 {
        Some(StatusTone::Running)
    } else if counts.waiting > 0 {
        Some(StatusTone::Waiting)
    } else {
        None
    }
}

fn status_tab_feature(tab: SidebarStatusTab) -> QueueFeature {
    match tab {
        SidebarStatusTab::AutoCompression => QueueFeature::Compression,
        SidebarStatusTab::Discord => QueueFeature::Discord,
        SidebarStatusTab::AiTagging => QueueFeature::Tagging,
    }
}

fn status_tab_label(tab: SidebarStatusTab) -> String {
    match tab {
        SidebarStatusTab::AutoCompression => tr!("sidebar.tab.auto_compression"),
        SidebarStatusTab::Discord => tr!("sidebar.tab.discord"),
        SidebarStatusTab::AiTagging => tr!("sidebar.tab.ai_tagging"),
    }
}

fn status_tab_element_id(tab: SidebarStatusTab) -> &'static str {
    match tab {
        SidebarStatusTab::AutoCompression => "sidebar-status-tab-auto-compression",
        SidebarStatusTab::Discord => "sidebar-status-tab-discord",
        SidebarStatusTab::AiTagging => "sidebar-status-tab-ai-tagging",
    }
}

pub(crate) fn scan_progress_label(progress: &ScanProgressView) -> String {
    match progress.phase {
        IpcScanPhase::Walking => tr_args!(
            "sidebar.scan_progress",
            root = progress.root,
            roots = progress.roots,
            count = progress.processed
        ),
        IpcScanPhase::Indexing => tr_args!(
            "sidebar.scan_index",
            root = progress.root,
            roots = progress.roots,
            indexed = progress.processed,
            total = progress
                .total
                .map(|total| total.to_string())
                .unwrap_or_else(|| "?".to_string())
        ),
    }
}

pub(crate) fn scan_progress_ratio(progress: &ScanProgressView) -> f32 {
    let ratio = match progress.phase {
        IpcScanPhase::Walking => 0.0,
        IpcScanPhase::Indexing => progress
            .total
            .filter(|total| *total > 0)
            .map(|total| progress.processed as f32 / total as f32)
            .unwrap_or(0.0),
    };
    ratio.clamp(0.0, 1.0)
}

fn nav_index(screen: Screen) -> usize {
    match screen {
        Screen::Photos => 0,
        Screen::Search => 1,
        Screen::Compression => 2,
        Screen::Dashboard => 3,
        Screen::Integrations => 4,
        Screen::Settings => 5,
    }
}

fn nav_item_top(index: usize) -> f32 {
    index as f32 * (NAV_ITEM_HEIGHT + NAV_ITEM_GAP)
}

fn nav_item_insets(reveal_progress: f32) -> (f32, f32) {
    let pad = 11.0 + (18.5 - 11.0) * reveal_progress;
    let gap = 11.0 * (1.0 - reveal_progress);
    (pad, gap)
}

/// Screen-space origin of the collapsed-sidebar tooltip. The menu starts at
/// the sidebar's top padding (there is no in-flow toggle row), so the tooltip
/// tracks the item top plus half of its height.
pub(crate) fn nav_tooltip_origin(index: usize) -> (f32, f32) {
    (
        WORKSPACE_PAD + sidebar_width(true) + 20.0,
        WORKSPACE_PAD + 16.0 + nav_item_top(index) + 8.5,
    )
}

/// Screen-space origin of the floating rail toggle. It straddles the sidebar's
/// right edge like the v0.7.0 control.
pub(crate) fn rail_toggle_origin(sidebar_collapsed: bool) -> (f32, f32) {
    (
        WORKSPACE_PAD + sidebar_width(sidebar_collapsed) - 12.0,
        WORKSPACE_PAD + 18.0,
    )
}

fn nav_bar_left(collapsed: bool) -> f32 {
    if collapsed { 5.0 } else { -1.0 }
}

pub(crate) fn screen_direction(from: Screen, to: Screen) -> f32 {
    if nav_index(to) >= nav_index(from) {
        1.0
    } else {
        -1.0
    }
}

/// Horizontal offset of the entering screen for a `0.0..=1.0` motion progress.
pub(crate) fn screen_enter_offset(direction: f32, progress: f32) -> f32 {
    direction * SCREEN_SLIDE * (1.0 - progress.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::{
        NAV_ITEM_GAP, NAV_ITEM_HEIGHT, StatusTone, nav_index, nav_item_insets, nav_item_top,
        nav_tooltip_origin, queue_attention, rail_toggle_origin, resolve_status_tab,
        scan_progress_label, scan_progress_ratio, screen_direction, screen_enter_offset,
        sidebar_status_tabs,
    };
    use crate::app::Screen;
    use crate::app::SidebarStatusTab;
    use crate::app::lifecycle::ScanProgressView;
    use crate::ui::theme::SCREEN_SLIDE;
    use vrc_photon_core::queue::QueueCounts;
    use vrc_photon_ipc::ScanPhase as IpcScanPhase;

    #[test]
    fn nav_index_matches_sidebar_order() {
        assert_eq!(nav_index(Screen::Photos), 0);
        assert_eq!(nav_index(Screen::Search), 1);
        assert_eq!(nav_index(Screen::Compression), 2);
        assert_eq!(nav_index(Screen::Dashboard), 3);
        assert_eq!(nav_index(Screen::Integrations), 4);
        assert_eq!(nav_index(Screen::Settings), 5);
    }

    #[test]
    fn nav_item_top_uses_item_height_and_gap() {
        assert_eq!(nav_item_top(0), 0.0);
        assert_eq!(nav_item_top(1), NAV_ITEM_HEIGHT + NAV_ITEM_GAP);
        assert_eq!(nav_item_top(4), 4.0 * (NAV_ITEM_HEIGHT + NAV_ITEM_GAP));
    }

    #[test]
    fn nav_item_insets_follow_sidebar_reveal_progress() {
        // Expanded: compact left inset with a gap between icon and label.
        assert_eq!(nav_item_insets(0.0), (11.0, 11.0));
        // Collapsed: wider inset centers the 17px icon in the 54px row, no label gap.
        assert_eq!(nav_item_insets(1.0), (18.5, 0.0));
        // Mid-animation keeps both values monotonic toward the collapsed state.
        assert_eq!(nav_item_insets(0.5), (14.75, 5.5));
    }

    #[test]
    fn nav_tooltip_origin_follows_the_top_aligned_menu() {
        assert_eq!(nav_tooltip_origin(0), (112.0, 38.5));
        assert_eq!(nav_tooltip_origin(1), (112.0, 81.5));
    }

    #[test]
    fn rail_toggle_origin_straddles_the_sidebar_edge() {
        assert_eq!(rail_toggle_origin(false), (240.0, 32.0));
        assert_eq!(rail_toggle_origin(true), (80.0, 32.0));
    }

    #[test]
    fn screen_direction_follows_sidebar_order() {
        assert_eq!(screen_direction(Screen::Photos, Screen::Settings), 1.0);
        assert_eq!(screen_direction(Screen::Settings, Screen::Photos), -1.0);
        assert_eq!(screen_direction(Screen::Search, Screen::Search), 1.0);
    }

    #[test]
    fn screen_enter_offset_follows_the_motion_progress() {
        assert_eq!(screen_enter_offset(1.0, 0.0), SCREEN_SLIDE);
        assert_eq!(screen_enter_offset(-1.0, 0.0), -SCREEN_SLIDE);
        assert_eq!(screen_enter_offset(1.0, 0.5), SCREEN_SLIDE / 2.0);
        assert_eq!(screen_enter_offset(-1.0, 0.5), -SCREEN_SLIDE / 2.0);
        assert_eq!(screen_enter_offset(1.0, 1.0), 0.0);
        assert_eq!(screen_enter_offset(-1.0, 1.0), 0.0);
        assert_eq!(screen_enter_offset(1.0, 2.0), 0.0);
    }
    #[test]
    fn scan_progress_label_uses_root_and_counts() {
        let progress = ScanProgressView {
            phase: IpcScanPhase::Indexing,
            processed: 50,
            total: Some(100),
            indexed: 49,
            root: 2,
            roots: 3,
        };
        assert_eq!(scan_progress_label(&progress), "フォルダ 2/3 · 索引 50/100");
        assert_eq!(scan_progress_ratio(&progress), 0.5);
    }

    #[test]
    fn sidebar_status_tabs_follow_feature_switches() {
        use SidebarStatusTab::{AiTagging, AutoCompression, Discord};

        assert_eq!(
            sidebar_status_tabs(true, true, true),
            vec![AutoCompression, Discord, AiTagging]
        );
        assert_eq!(sidebar_status_tabs(false, true, false), vec![Discord]);
        assert_eq!(sidebar_status_tabs(false, false, true), vec![AiTagging]);
        assert!(sidebar_status_tabs(false, false, false).is_empty());
    }

    #[test]
    fn resolve_status_tab_falls_back_to_the_first_visible() {
        use SidebarStatusTab::{AiTagging, AutoCompression, Discord};

        let all = [AutoCompression, Discord, AiTagging];
        assert_eq!(resolve_status_tab(Discord, &all), Some(Discord));
        assert_eq!(
            resolve_status_tab(AiTagging, &[AutoCompression, Discord]),
            Some(AutoCompression)
        );
        assert_eq!(resolve_status_tab(Discord, &[]), None);
    }

    #[test]
    fn queue_attention_prioritizes_active_over_waiting() {
        let counts = |waiting, pending, processing| QueueCounts {
            waiting,
            pending,
            processing,
        };

        assert_eq!(queue_attention(counts(1, 0, 1)), Some(StatusTone::Running));
        assert_eq!(queue_attention(counts(1, 1, 0)), Some(StatusTone::Running));
        assert_eq!(queue_attention(counts(1, 0, 0)), Some(StatusTone::Waiting));
        assert_eq!(queue_attention(counts(0, 0, 0)), None);
    }

    #[test]
    fn tone_colors_use_the_sidebar_palette() {
        let p = crate::ui::palette::resolve("charcoal", "sky");
        assert_eq!(StatusTone::Running.color(&p), p.accent);
        assert_eq!(StatusTone::Waiting.color(&p), p.amber);
    }
}
