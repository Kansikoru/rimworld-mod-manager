use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;
use std::io::Cursor;
use std::path::{Path, PathBuf};

pub fn get_default_mods_config_path() -> Option<PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let appdata_path = Path::new(&appdata);
        // %APPDATA%\..\LocalLow\...
        if let Some(parent) = appdata_path.parent() {
            let config_path = parent
                .join("LocalLow")
                .join("Ludeon Studios")
                .join("RimWorld by Ludeon Studios")
                .join("Config")
                .join("ModsConfig.xml");
            if config_path.exists() {
                return Some(config_path);
            }
        }
    }

    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let config_path = Path::new(&userprofile)
            .join("AppData")
            .join("LocalLow")
            .join("Ludeon Studios")
            .join("RimWorld by Ludeon Studios")
            .join("Config")
            .join("ModsConfig.xml");
        if config_path.exists() {
            return Some(config_path);
        }
    }

    None
}

pub fn load_active_mods(config_path: &Path) -> Vec<String> {
    if !config_path.exists() {
        return Vec::new();
    }

    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut reader = Reader::from_str(&content);
    reader.config_mut().trim_text(true);

    let mut active_mods = Vec::new();
    let mut in_active_mods = false;
    let mut current_tag = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if tag.eq_ignore_ascii_case("activemods") {
                    in_active_mods = true;
                }
                current_tag = tag;
            }
            Ok(Event::End(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if tag.eq_ignore_ascii_case("activemods") {
                    in_active_mods = false;
                }
                current_tag.clear();
            }
            Ok(Event::Text(ref e)) => {
                if in_active_mods && current_tag.eq_ignore_ascii_case("li") {
                    let text = e.unescape().unwrap_or_default().trim().to_string();
                    if !text.is_empty() {
                        active_mods.push(text);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    active_mods
}

pub fn save_active_mods(config_path: &Path, active_mods: &[String]) -> Result<(), String> {
    if !config_path.exists() {
        return Err(format!("Файл конфига не найден: {:?}", config_path));
    }

    let backup_path = config_path.with_extension("xml.bak");
    let _ = std::fs::copy(config_path, &backup_path);

    let content = std::fs::read_to_string(config_path)
        .map_err(|e| format!("Ошибка чтения ModsConfig.xml: {}", e))?;

    let mut reader = Reader::from_str(&content);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();

    let mut in_active_mods = false;
    let mut found_active_mods = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if tag.eq_ignore_ascii_case("activemods") {
                    in_active_mods = true;
                    found_active_mods = true;
                    // Write new activeMods block
                    let start = BytesStart::new("activeMods");
                    writer
                        .write_event(Event::Start(start.borrow()))
                        .map_err(|e| e.to_string())?;

                    for m in active_mods {
                        writer
                            .write_event(Event::Start(BytesStart::new("li")))
                            .map_err(|e| e.to_string())?;
                        writer
                            .write_event(Event::Text(BytesText::new(m)))
                            .map_err(|e| e.to_string())?;
                        writer
                            .write_event(Event::End(BytesEnd::new("li")))
                            .map_err(|e| e.to_string())?;
                    }
                } else if !in_active_mods {
                    writer
                        .write_event(Event::Start(e.clone()))
                        .map_err(|e| e.to_string())?;
                }
            }
            Ok(Event::End(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if tag.eq_ignore_ascii_case("activemods") {
                    in_active_mods = false;
                    writer
                        .write_event(Event::End(BytesEnd::new("activeMods")))
                        .map_err(|e| e.to_string())?;
                } else if !in_active_mods {
                    if tag.eq_ignore_ascii_case("modsconfigdata") && !found_active_mods {
                        // Inject activeMods before closing root tag
                        let start = BytesStart::new("activeMods");
                        writer.write_event(Event::Start(start.borrow())).unwrap();
                        for m in active_mods {
                            writer.write_event(Event::Start(BytesStart::new("li"))).unwrap();
                            writer.write_event(Event::Text(BytesText::new(m))).unwrap();
                            writer.write_event(Event::End(BytesEnd::new("li"))).unwrap();
                        }
                        writer.write_event(Event::End(BytesEnd::new("activeMods"))).unwrap();
                    }
                    writer
                        .write_event(Event::End(e.clone()))
                        .map_err(|e| e.to_string())?;
                }
            }
            Ok(Event::Empty(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if tag.eq_ignore_ascii_case("activemods") {
                    found_active_mods = true;
                    let start = BytesStart::new("activeMods");
                    writer
                        .write_event(Event::Start(start.borrow()))
                        .map_err(|e| e.to_string())?;

                    for m in active_mods {
                        writer
                            .write_event(Event::Start(BytesStart::new("li")))
                            .map_err(|e| e.to_string())?;
                        writer
                            .write_event(Event::Text(BytesText::new(m)))
                            .map_err(|e| e.to_string())?;
                        writer
                            .write_event(Event::End(BytesEnd::new("li")))
                            .map_err(|e| e.to_string())?;
                    }
                    writer
                        .write_event(Event::End(BytesEnd::new("activeMods")))
                        .map_err(|e| e.to_string())?;
                } else if !in_active_mods {
                    writer.write_event(Event::Empty(e.clone())).map_err(|e| e.to_string())?;
                }
            }
            Ok(Event::Eof) => break,
            Ok(event) => {
                if !in_active_mods {
                    writer.write_event(event).map_err(|e| e.to_string())?;
                }
            }
            Err(e) => return Err(format!("Ошибка парсинга XML: {}", e)),
        }
        buf.clear();
    }

    let result_bytes = writer.into_inner().into_inner();
    std::fs::write(config_path, result_bytes)
        .map_err(|e| format!("Ошибка записи ModsConfig.xml: {}", e))?;

    Ok(())
}
