with open('src/i18n.rs', 'r', encoding='utf-8') as f:
    content = f.read()

content = content.replace("?? ", "")
content = content.replace("?? ", "")

with open('src/i18n.rs', 'w', encoding='utf-8') as f:
    f.write(content)

with open('src/app.rs', 'r', encoding='utf-8') as f:
    app_rs = f.read()

app_rs = app_rs.replace('format!("?? {}", self.i18n.get(TranslationKey::AppPlayBtn))', 'self.i18n.get(TranslationKey::AppPlayBtn)')

with open('src/app.rs', 'w', encoding='utf-8') as f:
    f.write(app_rs)
