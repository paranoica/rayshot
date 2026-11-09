[English](comparison.md) · [Русский](comparison.ru.md)

# Сравнение с другими

rayshot делает одну вещь: сценарий Lightshot (клавиша, замерший экран,
выделение, рисование, Enter) с родной скоростью в GNOME на Wayland. Другие
программы умеют больше в других направлениях, и таблицы это показывают.

## В Linux

| | rayshot | Скриншотер GNOME | Flameshot | Ksnip | Spectacle | Gradia | WayShot | Satty / Swappy |
|---|---|---|---|---|---|---|---|---|
| GNOME на Wayland | да | да | экспериментально | да, с ограничениями | не описано | да, через портал | да, через портал | нет, только wlroots |
| Неподвижная картинка для выделения | в момент нажатия, ≈2 мс | в момент нажатия | после вызова портала | после вызова портала | не описано для GNOME | встроенный скриншотер GNOME | встроенный скриншотер GNOME | — |
| Стрелки, текст, фигуры | да | нет | да | да | да | да | да | да |
| Размытие / пикселизация | оба | нет | пикселизация | да | да | инструмент цензуры | оба | Satty оба, Swappy размытие |
| Загрузка, OCR | OCR на вашем компьютере, без загрузки | нет | Imgur | Imgur, OCR плагином | OCR | загрузка, OCR | нет | нет |
| От клавиши до выделения | ≈40 мс, замерено | замирает сразу, не замерялось | в GNOME жалуются на долгие задержки | не найдено | 1.5–2 с по отзывам | не найдено | не найдено | — |
| Память | 24 МБ своих, 146 МБ RSS | часть шелла | 36 МБ на Raspberry Pi | не найдено | 67 МБ на Raspberry Pi | не найдено | не найдено | не найдено |

<details>
<summary>Подробности и источники</summary>

- **Скриншотер GNOME** замораживает экран тем же вызовом композитора, что и rayshot (`screenshot.js` в GNOME Shell), но умеет только обрезать: рисовать нечем.
- **Flameshot:** поддержка GNOME на Wayland помечена как экспериментальная, снимает через портал, а окно снимка должно оставаться открытым ради буфера обмена ([README](https://github.com/flameshot-org/flameshot), [заметки к v14](https://github.com/flameshot-org/flameshot/releases)). В GNOME жалуются на окно разрешения и долгие задержки ([форум NixOS](https://discourse.nixos.org/t/looking-for-a-gnome-compatible-screenshot-annotation-tool/75964)), а Arch Wiki отмечает медленное выделение в некоторых версиях ([wiki](https://wiki.archlinux.org/title/Flameshot)).
- **Ksnip:** глобальные горячие клавиши на Wayland не работают, в GNOME для буфера обмена может понадобиться `QT_QPA_PLATFORM=xcb`; у проекта один мейнтейнер ([README](https://github.com/ksnip/ksnip)).
- **Spectacle:** программа KDE; пользователи пишут о 1.5–2 с от PrtSc до окна ([форум KDE](https://discuss.kde.org/t/make-spectacle-launch-faster/38030)).
- **Shutter:** в режиме поддержки, Wayland поддерживается в зачаточном виде ([FAQ](https://shutter-project.org/faq-help/)).
- **Gradia:** приложение на GTK4, чтобы украшать и закрашивать скриншоты, с OCR и загрузкой ([репозиторий](https://github.com/AlexanderVanhee/Gradia)).
- **WayShot:** редактор на GTK4 во Flathub; снимает через портал и открывает снимок в окне, умеет открывать файлы и вставленные картинки и сохранять в несколько форматов ([репозиторий](https://github.com/gutopardini/wayshot)).
- **Satty / Swappy:** редакторы для снимков `grim` на композиторах wlroots ([Satty](https://github.com/Satty-org/Satty), [Swappy](https://github.com/jtheoof/swappy)).
- Цифры памяти Flameshot и Spectacle взяты из теста на Raspberry Pi 4 ([LinuxLinks](https://www.linuxlinks.com/raspberry-pi-4-chronicling-desktop-experience-screen-capture/)). Контролируемых бенчмарков этих программ никто не публиковал; «не найдено» значит ровно это.

</details>

## Против классики Windows и macOS

| | rayshot | Lightshot | ShareX | Ножницы | macOS ⇧⌘4 | CleanShot X | Shottr |
|---|---|---|---|---|---|---|---|
| Платформа | Linux (GNOME) | Windows, macOS | Windows | Windows | macOS | macOS | macOS |
| Цена | бесплатно, MIT | бесплатно | бесплатно, GPL | встроено | встроено | $35 | сколько заплатите |
| Замораживает экран при нажатии | да | не описано | не описано | затемняет экран | не описано | не описано | по желанию |
| Рисовать до копирования | да | да | да | потом, в отдельном окне | в редакторе миниатюры | да | да |
| Размытие / пикселизация | оба | нет | оба | скрытие данных | нет | скрытие данных | оба | не найдено |
| Загрузка | никогда | публичные ссылки prnt.sc | много сервисов | нет | нет | облако | S3 |
| OCR, длинные снимки с прокруткой | OCR | нет | да | OCR | нет | да | да |
| Опубликованная скорость | ≈40 мс, замерено | не найдено | не найдено | не найдено | не найдено | не найдено | 17 мс снимок, 165 мс до показа (по словам автора) |

<details>
<summary>Подробности и источники</summary>

- **Lightshot** — оттуда и пришёл этот сценарий. Загруженные в нём картинки публичные: любой, кто угадает ссылку `prnt.sc`, их увидит ([политика конфиденциальности](https://lightshot.app/en/privacy.html)). rayshot никуда ничего не загружает.
- **ShareX** — самая полная бесплатная программа для Windows: пометки, много сервисов для загрузки, OCR, длинные снимки ([сайт](https://getsharex.com/), [выделение области](https://getsharex.com/docs/region-capture)).
- **Ножницы** затемняют экран, а снимок потом открывают в отдельном редакторе; OCR работает на самом устройстве ([Microsoft](https://support.microsoft.com/en-us/windows/use-snipping-tool-to-capture-screenshots-00246869-1843-655f-f220-97299b865f6b)).
- **macOS** редактирует через миниатюру в углу; с зажатым Control снимок идёт в буфер обмена ([Apple](https://support.apple.com/en-us/102646)).
- **CleanShot X:** $35 с годом обновлений, OCR на устройстве, длинные снимки, облако ([цены](https://cleanshot.com/pricing), [возможности](https://cleanshot.com/)).
- **Shottr** с версии 1.9 умеет замораживать экран при выделении области и заявляет 17 мс на снимок и около 165 мс до показа ([сайт](https://shottr.cc/), [заморозка](https://shottr.cc/kb/customareacapture)).
- **Greenshot** (Windows бесплатно, macOS $1.99) и **Snagit** (по подписке) тоже популярны, но про заморозку и скорость у них ничего не сказано ([Greenshot](https://getgreenshot.org/downloads/), [Snagit](https://www.techsmith.com/store/snagit)).

</details>

Назад к [README](../README.ru.md).
