use std::{
    alloc::{dealloc, Layout},
    env, fs,
    path::Path,
    rc::Rc,
    thread,
    time::Duration,
};

use cpp_core::{CppDeletable, Ptr, Ref, StaticUpcast};
use qt_core::{
    slot, ApplicationAttribute, QBox, QCoreApplication, QObject, QPtr, QString, SlotNoArgs,
    SlotOfQString,
};
use qt_gui::QGuiApplication;
use qt_ui_tools::QUiLoader;
use qt_widgets::{QApplication, QGridLayout, QLineEdit, QPushButton, QWidget};
use regex::{escape, Regex};
use serde::{Deserialize, Serialize};

const UI: &[u8] = include_bytes!("ui/main.ui");

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Kaomoji {
    text: String,
    name: String,
    search_tags: Vec<String>,
}

impl PartialEq for Kaomoji {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
    fn ne(&self, other: &Self) -> bool {
        self.name != other.name
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct Config {
    kaomojis: Vec<Kaomoji>,
}

struct Form {
    app: Ptr<QApplication>,
    widget: QBox<QWidget>,
    scroll_area: QBox<QWidget>,
    search_bar: QBox<QLineEdit>,
    kaomojis: *mut Vec<Kaomoji>,
}

impl StaticUpcast<QObject> for Form {
    unsafe fn static_upcast(ptr: Ptr<Self>) -> Ptr<QObject> {
        ptr.widget.as_ptr().static_upcast()
    }
}

impl Form {
    fn new(a: Ptr<QApplication>) -> Rc<Self> {
        unsafe {
            let loader = QUiLoader::new_0a();
            let widget = loader.load_bytes(&UI);
            drop(loader);
            let scroll_area: QPtr<QWidget> = widget.find_child("scrollAreaWidgetContents").unwrap();
            let search_bar: QPtr<QLineEdit> = widget.find_child("searchBar").unwrap();
            let kaomojis = Box::into_raw(Box::new(Vec::new())); // What the fuck is this
                                                                // Thanks copilot for the suggestion.
            let this = Rc::new(Self {
                app: a,
                widget,
                scroll_area: QBox::from_q_ptr(scroll_area),
                search_bar: QBox::from_q_ptr(search_bar),
                kaomojis,
            });
            this.init();
            return this;
        }
    }
    fn init(self: &Rc<Self>) {
        unsafe {
            self.app
                .about_to_quit()
                .connect(&self.slot_on_process_finished());
            self.search_bar
                .text_changed()
                .connect(&self.slot_on_search_bar_changed());
        }
    }
    fn parse_config(self: &Rc<Self>, path: &str) -> Config {
        let config_str = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error reading config file: {}", e);
                unsafe {
                    QCoreApplication::exit_1a(1);
                }
                self.exit_process(1)
            }
        }
        .to_lowercase();
        let config: Config = match serde_json::from_str(&config_str) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error parsing config file: {}", e);
                unsafe {
                    QCoreApplication::exit_1a(1);
                }
                self.exit_process(1)
            }
        };
        drop(config_str);
        // Detect duplicates in the config
        let mut names = Vec::new();
        for kaomoji in config.kaomojis.iter() {
            if names.contains(&kaomoji.name) {
                eprintln!("Duplicate kaomoji name in config: {}", kaomoji.name);
                unsafe {
                    QCoreApplication::exit_1a(1);
                }
            }
            names.push(kaomoji.name.clone());
        }
        drop(names);
        config
    }
    /// Populates the form with all of the kaomojis it can find
    /// in the config file given by `path`.
    fn populate(self: &Rc<Self>, path: Option<&str>, kaomojis: Option<Vec<Kaomoji>>) {
        let layout: QPtr<QGridLayout> = unsafe { self.scroll_area.layout().dynamic_cast() }; // C++ black magic
        unsafe {
            self.remove_widgets();
        }
        let config: Config;
        match path {
            Some(p) => {
                config = self.parse_config(p);
                for kaomoji in config.kaomojis.iter() {
                    unsafe {
                        // Please don't leak memory
                        (*(self.kaomojis)).push(kaomoji.clone());
                    }
                }
            }
            None => {
                config = Config {
                    kaomojis: match kaomojis {
                        Some(k) => k,
                        None => {
                            panic!("Neither path nor kaomojis were given!");
                        }
                    },
                }
            }
        }
        let mut row = 0;
        let mut column = 0;
        for kaomoji in config.kaomojis.into_iter() {
            unsafe {
                let button = QPushButton::new();
                button.set_flat(true);
                button.set_text(&QString::from_std_str(&kaomoji.text));
                button.set_object_name(&QString::from_std_str(&kaomoji.name));
                self.on_button_clicked_glue(QPtr::from_raw(button.as_raw_ptr()));
                layout.add_widget_3a(&button, row, column);
                column += 1;
                if column == 2 {
                    column = 0;
                    row += 1;
                }
                drop(button);
            }
        }
    }
    unsafe fn on_button_clicked_glue(self: &Rc<Self>, button: QPtr<QPushButton>) {
        let buttonptr2 = button.clone();
        let closure = SlotNoArgs::new(&button, move || {
            let clipboard = QGuiApplication::clipboard();
            clipboard.set_text_1a(&buttonptr2.text());
            thread::sleep(Duration::from_millis(2000));
            QCoreApplication::exit_1a(0);
        });
        button.clicked().connect(&closure);
    }

    #[slot(SlotOfQString)]
    unsafe fn on_search_bar_changed(self: &Rc<Self>, text: Ref<QString>) {
        if text.to_std_string().is_empty() {
            self.populate(None, Some((*(self.kaomojis)).clone()));
            return;
        }
        let temp = escape(text.to_std_string().as_str().trim()).to_lowercase();
        let tag_matches = temp.split(" ").collect::<Vec<&str>>();

        let (tags, more_than_one_tag) = match tag_matches.len() {
            1 => (tag_matches, false),
            _ => (tag_matches, true),
        };
        let mut regxs: Vec<(&str, bool)> = Vec::new();
        for tag in tags {
            regxs.push((tag, false))
        }
        let re = Regex::new(format!("{}{}{}", ".*(", &temp, ").*").as_str()).unwrap(); // Surely it is very unlikely to error, right?
        let mut matches: Vec<Kaomoji> = Vec::new();
        for kaomoji in (*(self.kaomojis)).iter() {
            if !more_than_one_tag {
                if re.is_match(kaomoji.name.as_str()) {
                    if matches.contains(kaomoji) {
                        continue;
                    }
                    matches.push(kaomoji.clone());
                    continue;
                }
                if re.is_match(kaomoji.text.as_str()) {
                    if matches.contains(kaomoji) {
                        continue;
                    }
                    matches.push(kaomoji.clone());
                    continue;
                }
            }
            // check if any of the search tags match
            for tag in kaomoji.search_tags.iter() {
                for reg in regxs.iter_mut() {
                    println!("Checking tag {} against {:?}", tag, reg);
                    if reg.1 {
                        continue;
                    }
                    if tag.contains(reg.0) {
                        reg.1 = true
                    }
                }
            }
            let mut all_match: bool = true;
            for reg in regxs.iter() {
                println!("{}", !reg.1);
                if !(reg.1) {
                    println!("{} didn't match", reg.0);
                    all_match = false
                }
            }
            println!("check results: {:#?}, aaaa: {}", regxs, all_match);
            if all_match {
                println!("all matched");
                if matches.contains(kaomoji) {
                    continue;
                }
                matches.push(kaomoji.clone());
            }
            for reg in regxs.iter_mut() {
                reg.1 = false;
            }
        }
        self.remove_widgets();
        println!("Matches: {:#?}", matches);
        self.populate(None, Some(matches));
        drop(re);
    }

    #[slot(SlotNoArgs)]
    unsafe fn on_process_finished(self: Rc<Self>) {
        // Very nice way to free a pointer, rust. /s
        self.kaomojis.drop_in_place();
        dealloc(self.kaomojis as *mut u8, Layout::new::<Vec<Kaomoji>>());
        drop(self);
    }

    unsafe fn remove_widgets(self: &Rc<Self>) {
        let layout: QPtr<QGridLayout> = self.scroll_area.layout().dynamic_cast();
        let mut widgets = Vec::new();
        for i in 0..layout.count() {
            widgets.push(layout.item_at(i).widget());
        }
        for widget in widgets {
            layout.remove_widget(&widget);
            widget.delete();
            drop(widget);
        }
        drop(layout);
    }

    fn exit_process(self: &Rc<Self>, exit_code: i32) -> ! {
        println!("Exiting with code {}", exit_code);
        unsafe {
            self.kaomojis.drop_in_place();
            dealloc(self.kaomojis as *mut u8, Layout::new::<Vec<Kaomoji>>());
            drop(self);
            std::process::exit(exit_code);
        }
    }
}
fn main() {
    unsafe { QCoreApplication::set_attribute_1a(ApplicationAttribute::AAShareOpenGLContexts) } // So it can stop yelling at me.
    QApplication::init(|a| unsafe {
        let form = Form::new(a);
        let home_path = env::var("HOME").unwrap();
        let base_config = Path::new("/etc/kaomoji-picker.json");
        let temp = format!("{}{}", &home_path, "/.config/kaomoji-picker.json");
        let user_config = Path::new(temp.as_str());
        match user_config.exists() {
            true => form.populate(Some(user_config.to_str().unwrap()), None),
            false => match base_config.exists() {
                true => form.populate(Some(base_config.to_str().unwrap()), None),
                false => {
                    eprintln!("No config file found in /etc or ~/.config");
                    std::process::exit(1);
                }
            },
        }
        drop(temp);
        form.widget.show();
        QApplication::exec()
    })
}
