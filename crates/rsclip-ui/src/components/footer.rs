use gtk::prelude::*;
use gtk4 as gtk;

pub(crate) struct FooterBar {
    pub(crate) container: gtk::Box,
    pub(crate) footer: gtk::Label,
    pub(crate) ocr_button: gtk::Button,
}

pub(crate) fn build() -> FooterBar {
    let container = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    container.set_valign(gtk::Align::Center);
    container.add_css_class("footer");

    let footer = gtk::Label::new(Some(
        "Tab: switch tab | Enter: paste | Ctrl+C: copy | Ctrl+S: secret | Ctrl+P: pin | Ctrl+D: delete | Esc: close",
    ));
    footer.add_css_class("footer-label");
    footer.add_css_class("muted");
    footer.set_xalign(0.0);
    footer.set_single_line_mode(true);
    footer.set_ellipsize(gtk::pango::EllipsizeMode::End);
    footer.set_wrap(false);
    footer.set_hexpand(true);
    container.append(&footer);

    let ocr_button = gtk::Button::with_label("OCR");
    ocr_button.add_css_class("ocr-button");
    ocr_button.set_opacity(0.0);
    ocr_button.set_sensitive(false);
    container.append(&ocr_button);

    FooterBar {
        container,
        footer,
        ocr_button,
    }
}
