use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{
    TitleBar,
    breadcrumb::{Breadcrumb, BreadcrumbItem},
    divider::Divider,
    h_flex,
    sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem, SidebarToggleButton},
    skeleton::Skeleton,
    v_flex, ActiveTheme, Icon, Root, Sizable, StyledExt,
};
mod icons;
use icons::Assets;

/// Erstellt einen Icon aus einem beliebigen SVG-Dateinamen aus `src/icons/`.
/// Beispiel: `icon("chart-line")` lädt `icons/chart-line.svg`.
fn icon(name: &'static str) -> Icon {
    Icon::default().path(format!("icons/{name}.svg"))
}

// ── App ───────────────────────────────────────────────────────────────────────

struct Page {
    sidebar_collapsed: bool,
}

impl Page {
    fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            sidebar_collapsed: false,
        }
    }

    fn toggle_sidebar(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }
}

impl Render for Page {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let collapsed = self.sidebar_collapsed;

        // ── Sidebar ──────────────────────────────────────────────────────────
        let sidebar = Sidebar::left()
            .bg(cx.theme().sidebar_accent)
            .collapsed(collapsed)
            .header(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .size_8()
                            .rounded_lg()
                            .bg(cx.theme().primary)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(cx.theme().primary_foreground)
                            .child(Icon::new(icon("bot")).large()),
                    )
                    .when(!collapsed, |this| {
                        this.child(
                            v_flex()
                                .child(div().text_sm().font_semibold().child("TradingBot"))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Paper Trading"),
                                ),
                        )
                    }),
            )
            .child(
                SidebarGroup::new("Platform").child(
                    SidebarMenu::new()
                        .child(
                            SidebarMenuItem::new("Dashboard")
                                .icon(icon("house"))
                                .active(true),
                        )
                        .child(SidebarMenuItem::new("Portfolio").icon(icon("chart-line")))
                        .child(SidebarMenuItem::new("Strategies").icon(icon("settings")))
                        .child(SidebarMenuItem::new("History").icon(icon("history"))),
                ),
            );

        // ── Layout ────────────────────────────────────────────────────────────
        v_flex()
            .size_full()
            .bg(cx.theme().sidebar_accent)
            // Custom title bar (replaces native macOS bar)
            .child(TitleBar::new().child(
                h_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("TradingBot"),
            ))
            // Body row
            .child(
        h_flex()
            .size_full()
            .bg(cx.theme().sidebar_accent)
            .text_color(cx.theme().foreground)
            // Sidebar
            .child(sidebar)
            // SidebarInset: padded outer area → rounded white/dark box inside
            .child(
                // Padding container — this creates the gap visible around the box
                div().flex_1().min_w_0().h_full().p_2().pl_0().child(
                    // The actual inset box with rounded corners
                    v_flex()
                        .size_full()
                        .rounded_xl()
                        .bg(cx.theme().background)
                        .overflow_hidden()
                        // Header
                        .child(
                            h_flex()
                                .h_16()
                                .flex_shrink_0()
                                .items_center()
                                .gap_2()
                                .px_4()
                                .child(
                                    SidebarToggleButton::left()
                                        .collapsed(collapsed)
                                        .on_click(cx.listener(Self::toggle_sidebar)),
                                )
                                .child(Divider::vertical().h_4())
                                .child(
                                    Breadcrumb::new()
                                        .child(BreadcrumbItem::new("Build Your Application"))
                                        .child(BreadcrumbItem::new("Data Fetching")),
                                ),
                        )
                        // Content
                        .child(
                            v_flex()
                                .flex_1()
                                .gap_4()
                                .p_4()
                                .pt_0()
                                .child(
                                    h_flex()
                                        .gap_4()
                                        .child(Skeleton::new().flex_1().h(px(160.)).rounded_xl())
                                        .child(Skeleton::new().flex_1().h(px(160.)).rounded_xl())
                                        .child(Skeleton::new().flex_1().h(px(160.)).rounded_xl()),
                                )
                                .child(Skeleton::new().w_full().flex_1().rounded_xl()),
                        ),
                ),
            ) // end body h_flex
            ) // end outer v_flex
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        gpui_component::init(cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(1200.), px(800.)),
                })),
                titlebar: Some(TitleBar::title_bar_options()),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| Page::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .unwrap();
    });
}
