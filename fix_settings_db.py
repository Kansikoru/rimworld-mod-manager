with open('src/ui/view_settings.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

for i, line in enumerate(lines):
    if "app.community_db_rules_count));" in line:
        lines[i] = '            ui.label(t.get(TranslationKey::SettingsDbCount).replace("{}", &app.community_db_rules_count.to_string()));\n'

with open('src/ui/view_settings.rs', 'w', encoding='utf-8') as f:
    f.writelines(lines)
