use rimworld_mod_manager::rimworld::workshop::load_all_mods;
fn main() {
    let mods = load_all_mods();
    
    // Find Adaptive Storage Framework
    if let Some(m) = mods.iter().find(|m| m.name.to_lowercase().contains("adaptive storage framework")) {
        println!("Mod: {} ({})", m.name, m.package_id);
        println!("Dependencies: {:?}", m.dependencies);
        println!("Load Before: {:?}", m.load_before);
        println!("Load After: {:?}", m.load_after);
    }
    println!("---");
    // Check all mods that depend on it
    for m in &mods {
        if m.dependencies.iter().any(|d| d.to_lowercase().contains("adaptive") || d.to_lowercase().contains("bustedbunny")) {
            println!("Depends on ASF: {} ({})", m.name, m.package_id);
        }
    }
}
