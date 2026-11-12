# Шрифты

`Inter-Regular.ttf` и `Inter-SemiBold.ttf` — [Inter 4.1](https://github.com/rsms/inter/releases/tag/v4.1)
(SIL Open Font License 1.1, текст в `Inter-LICENSE.txt`) без символов области Private Use Area (U+E000–U+F8FF):
в ней лежат значки Phosphor, и символы Inter рисовались бы вместо них. Получено так:

```bash
pyftsubset Inter-Regular.ttf --unicodes="U+0000-DFFF,U+F900-FFFF,U+10000-10FFFF" --layout-features='*' --name-IDs='*' --notdef-outline --output-file=Inter-Regular.ttf
```
