<p align="center">
  <img src="assets/glance.png" width="96" height="96" alt="Glance">
</p>

<h1 align="center">Glance</h1>

<p align="center">
  Распознавание и перевод текста с экрана прямо из системного трея Windows.<br>
  Нажми сочетание клавиш, выдели любой текст на экране, а потом скопируй или переведи его.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/version-0.0.2--beta-7698fd?style=flat-square" alt="Версия 0.0.2-beta">
  <img src="https://img.shields.io/badge/Windows-10%20%7C%2011-0078d4?style=flat-square" alt="Windows 10 и 11">
  <img src="https://img.shields.io/badge/Rust-2026-dea584?style=flat-square&logo=rust&logoColor=white" alt="Rust 2026">
  <img src="https://img.shields.io/badge/UI-Slint-2379f4?style=flat-square" alt="Интерфейс на Slint">
  <a href="https://github.com/neodym121/glance/actions/workflows/build.yml"><img src="https://img.shields.io/github/actions/workflow/status/neodym121/glance/build.yml?style=flat-square&label=build" alt="Статус сборки"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/neodym121/glance?style=flat-square" alt="Лицензия MIT"></a>
</p>

<p align="center">
  <a href="README.md">English</a> · <b>Русский</b>
</p>

Glance — нативное приложение на Rust с интерфейсом на [Slint](https://slint.dev): без
WebView, Electron и браузерных движков. В трее занимает около **2 МБ памяти**.

## Возможности

- **Любой текст на экране**: картинки, видео, игры, удалённый рабочий стол, PDF с запретом
  копирования.
- **Распознавание** через Paddle OCR (офлайн и бесплатно; славянская модель читает
  русский, украинский и белорусский, есть и английская — обе один раз скачиваются из
  настроек), либо vision-модель от Google Gemini, Mistral AI, Groq или Polza AI.
- **Перевод в один клик** через ИИ-модель, Google Translate (без ключа), DeepL или
  Microsoft Translator.
- **Выбор модели**: поиск по названию или ID, обновление списка, любой свой ID модели.
- **Приватность**: ключи в диспетчере учётных данных Windows, скриншоты не пишутся на диск.

## Скачать

На странице [Releases](https://github.com/neodym121/glance/releases): установщик `.msi` или
портативный `.exe`. Нужна 64-битная Windows 10 или 11.

## Как пользоваться

1. Нажми **Ctrl+Alt+S** (или щёлкни по значку в трее) и выдели текст. **Esc** — отмена.
2. Распознанный текст появится в карточке и скопируется в буфер обмена.
3. **Translate** показывает перевод, **Open** открывает ссылку из текста.

Настройки открываются при первом запуске, потом — из меню в трее.

<p align="center">
  <img src="docs/settings.png" width="690" alt="Настройки Glance">
</p>

Paddle OCR при первом включении скачивает файлы моделей (~12 МБ на язык) —
в настройках, на странице «Recognition».

## API-ключи

Один ключ провайдера работает и для распознавания, и для перевода. Вставь его на странице
**API Keys** в настройках.

| Провайдер | Где взять ключ |
|---|---|
| Google Gemini | [aistudio.google.com/apikey](https://aistudio.google.com/apikey) |
| Mistral AI | [console.mistral.ai/api-keys](https://console.mistral.ai/api-keys) |
| Groq | [console.groq.com/keys](https://console.groq.com/keys) |
| Polza AI | [polza.ai/dashboard/api-keys](https://polza.ai/dashboard/api-keys) |
| Google Translate (необязательно) | [Google Cloud console](https://console.cloud.google.com), Cloud Translation API |
| DeepL | [deepl.com/your-account/keys](https://www.deepl.com/your-account/keys) |
| Microsoft Translator | [портал Azure](https://portal.azure.com), ресурс Translator |

## Сборка

Нужны stable Rust с тулчейном MSVC и Windows SDK.

```bash
cargo build --release
```

Установщик собирает [workflow Build](.github/workflows/build.yml) с помощью WiX.

## Лицензия

[MIT](LICENSE). Иконки: [Lucide](https://lucide.dev) (лицензия ISC).

---

<p align="center">
  <a href="https://slint.dev"><img src="ui/images/slint-logo-gray.png" height="32" alt="Slint"></a>
  <br>
  <sub>Glance сделан на Slint. Работает на вашей машине.</sub>
</p>
