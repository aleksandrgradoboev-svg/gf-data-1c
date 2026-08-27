<#
.SYNOPSIS
    Сборка расширения gt-data-1c. Одна сборка на любую базу.

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

.PARAMETER StripDefaultRole
    Не объявлять роль расширения основной ролью конфигурации.

    По умолчанию роль объявляется: иначе доступ к сервису есть только у того, кому её
    выдали руками, а обычный пользователь получает отказ прав, неотличимый от поломки
    канала. Флаг нужен для баз, где библиотека стандартных подсистем считает основные
    роли и лишняя запись роняет сеанс.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $OutputDir,
    [string] $CompatibilityMode = 'Version8_3_18',
    [switch] $StripDefaultRole
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$skills = Join-Path $env:USERPROFILE '.claude\skills'

if (Test-Path $OutputDir) { Remove-Item -Recurse -Force $OutputDir }

Write-Host "[1/6] Каркас расширения (режим совместимости $CompatibilityMode)"
& powershell.exe -NoProfile -File (Join-Path $skills 'cfe-init\scripts\cfe-init.ps1') `
    -Name 'GTData' -Synonym 'Доступ к данным' -NamePrefix 'GT_' `
    -Purpose AddOn -Version '0.1.0' -Vendor 'Aleksandr Gradoboev' `
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

if (-not $StripDefaultRole) {
    Write-Host "[5/6] Роль по умолчанию"
    & powershell.exe -NoProfile -File (Join-Path $skills 'cf-edit\scripts\cf-edit.ps1') `
        -ConfigPath $OutputDir -Operation add-defaultRole -Value 'GT_ОсновнаяРоль' | Out-Null
} else {
    Write-Host "[5/6] Роль по умолчанию — пропущена (-StripDefaultRole)"
}

Write-Host "[6/6] Модуль сервиса"
$moduleTarget = Join-Path $OutputDir 'HTTPServices\GT_Data\Ext\Module.bsl'
Copy-Item -Force (Join-Path $root 'extension\module\GT_Data.bsl') $moduleTarget

$lines = (Get-Content $moduleTarget).Count
Write-Host "OK: расширение собрано в $OutputDir (модуль: $lines строк, одна сборка на любую базу)"
