use gpui::{Div, div, prelude::*, px, rgb};

pub(crate) fn header(device_name: String, profile_name: String) -> Div {
    div()
        .h(px(54.0))
        .flex()
        .items_center()
        .justify_between()
        .pl(px(84.0))
        .pr_4()
        .child(
            div()
                .min_w_0()
                .flex()
                .items_center()
                .gap_3()
                .text_sm()
                .text_color(rgb(0xa7a7aa))
                .child(div().size(px(16.0)).rounded(px(5.0)).bg(rgb(0xd4d4d4)))
                .child(
                    div()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(device_name),
                ),
        )
        .child(
            div()
                .ml_4()
                .px_4()
                .py_2()
                .rounded(px(18.0))
                .bg(rgb(0x1b1b1b))
                .text_sm()
                .text_color(rgb(0xf1f1f1))
                .child(profile_name),
        )
}
