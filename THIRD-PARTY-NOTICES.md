# Сторонние компоненты

Продукт `gf-data-1c` (© 2026 Aleksandr Gradoboev, MIT) включает или использует перечисленное ниже.
Файл ведётся вместе с зависимостями: компонент без объявленной лицензии в сборку не принимается.

## Скомпонованные библиотеки Go

| Компонент | Версия | Лицензия |
|---|---|---|
| github.com/modelcontextprotocol/go-sdk | v1.4.0 | Apache-2.0 |
| github.com/google/jsonschema-go | v0.4.2 | MIT |
| github.com/segmentio/asm | v1.1.3 | MIT |
| github.com/segmentio/encoding | v0.5.3 | MIT |
| github.com/yosida95/uritemplate/v3 | v3.0.2 | BSD-3-Clause |
| golang.org/x/term, x/text, x/oauth2, x/sys | см. go.mod | BSD-3-Clause (The Go Authors) |

### Apache-2.0: MCP Go SDK

Копия лицензии — `third-party/apache-2.0.txt`; NOTICE донора сохраняется там же.
Изменения в код SDK не вносились: библиотека используется как есть, через публичное API.
Если это изменится, отметка о модификации ставится здесь (требование Apache-2.0 §4 b).

## Расширение информационной базы 1С

Расширение, устанавливаемое в базу (HTTP-сервис доступа к данным), разработано
Aleksandr Gradoboev с нуля и покрывается лицензией продукта. Стороннего кода не содержит.
