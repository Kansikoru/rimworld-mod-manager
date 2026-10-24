with open('src/ui/view_settings.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

# Line 14: Settings Subtitle
lines[13] = '        t.get(TranslationKey::SettingsSubtitle),\n'

with open('src/ui/view_settings.rs', 'w', encoding='utf-8') as f:
    f.writelines(lines)
