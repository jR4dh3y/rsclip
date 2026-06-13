use gtk::prelude::*;
use gtk4 as gtk;

pub(crate) struct Topbar {
    pub(crate) container: gtk::Box,
    pub(crate) history_button: gtk::Button,
    pub(crate) secrets_button: gtk::Button,
    pub(crate) search: gtk::SearchEntry,
    pub(crate) filter: gtk::DropDown,
    pub(crate) count: gtk::Label,
}

pub(crate) fn build(search_placeholder: &str) -> Topbar {
    let container = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    container.add_css_class("topbar");

    let history_button = gtk::Button::with_label("Clipboard");
    history_button.add_css_class("mode-button");
    history_button.add_css_class("active-mode");
    container.append(&history_button);

    let secrets_button = gtk::Button::with_label("Secrets");
    secrets_button.add_css_class("mode-button");
    container.append(&secrets_button);

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(search_placeholder));
    search.add_css_class("search-box");
    search.set_hexpand(true);
    container.append(&search);

    let filter =
        gtk::DropDown::from_strings(&["All", "Text", "Images", "Links", "Colors", "Pinned"]);
    filter.add_css_class("filter-select");
    filter.set_selected(0);
    container.append(&filter);

    let count = gtk::Label::new(Some("Entries 0"));
    count.add_css_class("topbar-count");
    count.set_width_chars(10);
    count.set_xalign(0.5);
    container.append(&count);

    Topbar {
        container,
        history_button,
        secrets_button,
        search,
        filter,
        count,
    }
}
