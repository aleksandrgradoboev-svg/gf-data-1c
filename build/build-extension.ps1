<#
.SYNOPSIS
    Сборка расширения gf-data-1c. Одна сборка на любую базу.

.DESCRIPTION
    Расширение намеренно НЕ заимствует язык расширяемой конфигурации. Это и делает его
    универсальным: собранное с языком, оно привязывается к UUID языка конкретной базы
    и в другую не встаёт («значение контролируемого свойства ОбъектРасширяемойКонфигурации
    у объекта Язык.Русский не совпадает»). Проверено 23.08.2026 на УТ и БП.

    Второе условие универсальности — низкий режим совместимости: он должен быть НЕ ВЫШЕ
    режима расширяемой конфигурации, иначе платформа отказывает при загрузке. Поэтому
    по умолчанию берётся заведомо старый режим, а не режим машины разработчика.

.PARAMETER OutputDir
    Куда положить готовые исходники расширения.

.PARAMETER CompatibilityMode
    Режим совместимости. Умолчание подобрано так, чтобы расширение вставало в типовые
    конфигурации последних лет; поднимать его нужно только ради возможностей платформы,
    которых нет в старом режиме.

.PARAMETER DefaultRole
    Объявить роль расширения ОСНОВНОЙ ролью конфигурации.

    Умолчание с 03.09.2026 — НЕ объявлять, и вот почему. Основная роль выдаётся
    платформой неявно: в списке ролей пользователя она не появляется никогда — ни
    когда работает, ни когда пропала. А в базах на БСП её стирает пересчёт ролей
    в подсистеме «Управление доступом», и канал молча умирает с отказом прав,
    неотличимым от неверного пароля. Разницы на экране при этом ноль.

    Видимая роль выдаётся в ПОЛЬЗОВАТЕЛЬСКОМ режиме через профиль группы доступа
    (Администрирование → Настройки пользователей и прав → Профили групп доступа):
    заводится дополнительный профиль с одной ролью GT_ОсновнаяРоль, по нему —
    группа доступа с нужными пользователями. Так право видно, проверяемо и
    переживает пересчёт ролей.

    Флаг нужен там, где профиль заводить негде: база без БСП, разовый стенд,
    демонстрация. Тогда доступ получают все пользователи сразу и без настройки.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $OutputDir,
    [string] $CompatibilityMode = 'Version8_3_18',
    # Версия расширения. Пусто — берётся из Cargo.toml, и это правильный путь:
    # probe сверяет версию расширения с версией сервера СТРОГО и при расхождении
    # объявляет ответы базы недостоверными. Два числа в разных файлах разъезжаются
    # молча, а вылезает это у пользователя, не у нас.
    [string] $Version,
    [switch] $DefaultRole
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$skills = Join-Path $env:USERPROFILE '.claude\skills'

if (-not $Version) {
    $cargo = Join-Path $root 'Cargo.toml'
    $строка = Select-String -Path $cargo -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if (-not $строка) { throw "версия не найдена в $cargo — назовите её параметром -Version" }
    $Version = $строка.Matches[0].Groups[1].Value
    Write-Host "[0/6] Версия расширения из Cargo.toml: $Version"
}

if (Test-Path $OutputDir) { Remove-Item -Recurse -Force $OutputDir }

Write-Host "[1/6] Каркас расширения (режим совместимости $CompatibilityMode)"
& powershell.exe -NoProfile -File (Join-Path $skills 'cfe-init\scripts\cfe-init.ps1') `
    -Name 'GTData' -Synonym 'Доступ к данным' -NamePrefix 'GT_' `
    -Purpose AddOn -Version $Version -Vendor 'Aleksandr Gradoboev' `
    -CompatibilityMode $CompatibilityMode -OutputDir $OutputDir | Out-Null

Write-Host "[2/6] Отвязка от языка расширяемой конфигурации"
$configXml = Join-Path $OutputDir 'Configuration.xml'
Remove-Item -Recurse -Force (Join-Path $OutputDir 'Languages')
$text = Get-Content $configXml -Raw -Encoding UTF8
$text = $text -replace '\s*<Language>Русский</Language>', ''
$text = $text -replace '\s*<DefaultLanguage>Language\.Русский</DefaultLanguage>', ''
[System.IO.File]::WriteAllText($configXml, $text, (New-Object System.Text.UTF8Encoding $true))

Write-Host "[3/6] HTTP-сервис"
& powershell.exe -NoProfile -File (Join-Path $skills 'meta-compile\scripts\meta-compile.ps1') `
    -JsonPath (Join-Path $root 'build\httpservice.json') -OutputDir $OutputDir | Out-Null

Write-Host "[4/6] Роль доступа"
& powershell.exe -NoProfile -File (Join-Path $skills 'role-compile\scripts\role-compile.ps1') `
    -JsonPath (Join-Path $root 'build\role.json') -OutputDir $OutputDir | Out-Null

if ($DefaultRole) {
    Write-Host "[5/6] Роль по умолчанию — объявляется (-DefaultRole)"
    & powershell.exe -NoProfile -File (Join-Path $skills 'cf-edit\scripts\cf-edit.ps1') `
        -ConfigPath $OutputDir -Operation add-defaultRole -Value 'GT_ОсновнаяРоль' | Out-Null
} else {
    # Флаг ОБЯЗАН удалять объявление, а не пропускать его добавление: каркас
    # cfe-init прописывает DefaultRoles сам, поэтому «просто не добавлять»
    # оставляло роль основной — и прежний флаг -StripDefaultRole, существовавший
    # ровно ради баз с БСП, молча не делал ничего. Поймано 03.09.2026 проверкой
    # собранного .cfe, а не доверием к сообщению скрипта.
    Write-Host "[5/6] Роль видимая — основной не объявляется (выдаётся профилем)"
    $confPath = Join-Path $OutputDir 'Configuration.xml'
    $text = [System.IO.File]::ReadAllText($confPath)
    $text = $text -replace '(?s)\s*<DefaultRoles>.*?</DefaultRoles>', ''
    [System.IO.File]::WriteAllText($confPath, $text, (New-Object System.Text.UTF8Encoding $true))
    if (Select-String -Path $confPath -Pattern 'DefaultRoles' -Quiet) {
        throw "DefaultRoles не удалось снять из $confPath"
    }
}

Write-Host "[6/6] Модуль сервиса"
$moduleTarget = Join-Path $OutputDir 'HTTPServices\GT_Data\Ext\Module.bsl'
Copy-Item -Force (Join-Path $root 'extension\module\GT_Data.bsl') $moduleTarget

$lines = (Get-Content $moduleTarget).Count
Write-Host "OK: расширение собрано в $OutputDir (модуль: $lines строк, одна сборка на любую базу)"
