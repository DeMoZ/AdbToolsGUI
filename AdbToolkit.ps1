# ADB Toolkit - утилита для диагностики и лечения подключения Android-устройств по USB.
# Запуск: AdbToolkit.bat (или powershell -STA -ExecutionPolicy Bypass -File AdbToolkit.ps1)
# Проверка без окна: powershell -STA -File AdbToolkit.ps1 -SelfTest
param([switch]$SelfTest)

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName Microsoft.VisualBasic

$script:Adb      = $null
$script:Serial   = $null
$script:Busy     = $false
$script:ServerUp = $false
$script:Devices  = @()
$script:DevSig   = ''
$script:Out      = $null

$script:ColText = [System.Drawing.Color]::FromArgb(220, 220, 220)
$script:ColHead = [System.Drawing.Color]::FromArgb(110, 200, 255)
$script:ColErr  = [System.Drawing.Color]::FromArgb(255, 120, 120)
$script:ColWarn = [System.Drawing.Color]::FromArgb(255, 200, 90)
$script:ColOk   = [System.Drawing.Color]::FromArgb(130, 220, 130)
$script:ColDim  = [System.Drawing.Color]::FromArgb(150, 150, 150)

# ---------------------------------------------------------------- базовые функции

function Find-Adb {
    $candidates = @(
        (Join-Path $PSScriptRoot 'platform-tools\adb.exe'),
        'C:\platform-tools\adb.exe',
        (Join-Path $env:LOCALAPPDATA 'Android\Sdk\platform-tools\adb.exe')
    )
    foreach ($p in $candidates) { if (Test-Path $p) { return $p } }
    $cmd = Get-Command adb.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    return $null
}

function Write-Log([string]$Text, $Color = $null) {
    if ($null -eq $Color) { $Color = $script:ColText }
    if ($null -eq $script:Out) { Write-Host $Text; return }
    $o = $script:Out
    $o.SelectionStart = $o.TextLength
    $o.SelectionLength = 0
    $o.SelectionColor = $Color
    $o.AppendText($Text + "`r`n")
    $o.ScrollToCaret()
}

function Join-Args([string[]]$ArgList) {
    ($ArgList | ForEach-Object {
        if ($_ -eq '' -or $_ -match '[\s"]') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ }
    }) -join ' '
}

# Запускает процесс, не блокируя окно. Возвращает Out / Err / Code / TimedOut.
function Invoke-Native([string]$File, [string]$Arguments, [int]$TimeoutSec = 30) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $File
    $psi.Arguments = $Arguments
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $psi.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    $p = [System.Diagnostics.Process]::Start($psi)
    $so = $p.StandardOutput.ReadToEndAsync()
    $se = $p.StandardError.ReadToEndAsync()
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSec)
    $timedOut = $false
    while (-not $p.WaitForExit(50)) {
        if ($script:Out) { [System.Windows.Forms.Application]::DoEvents() }
        if ([DateTime]::UtcNow -gt $deadline) {
            try { $p.Kill() } catch {}
            $timedOut = $true
            break
        }
    }
    $out = ''; $err = ''
    if ($so.Wait(1500)) { $out = $so.Result }
    if ($se.Wait(1500)) { $err = $se.Result }
    $code = -1
    if (-not $timedOut) { $code = $p.ExitCode }
    [pscustomobject]@{ Out = $out.TrimEnd(); Err = $err.TrimEnd(); Code = $code; TimedOut = $timedOut }
}

# adb start-server оставляет демона, который держит каналы вывода, поэтому запускаем его через cmd с перенаправлением в nul.
function Start-AdbServer {
    $null = Invoke-Native 'cmd.exe' ('/c ""{0}" start-server >nul 2>&1"' -f $script:Adb) 20
    $script:ServerUp = $true
}

function Invoke-Adb {
    param([string[]]$ArgList, [switch]$NoDevice, [switch]$Quiet, [int]$Timeout = 30)
    if (-not $script:Adb) { Write-Log 'adb.exe не найден. Укажите путь к нему сверху.' $script:ColErr; return $null }
    $full = @()
    if (-not $NoDevice) {
        if (-not $script:Serial) { Write-Log 'Сначала выберите устройство в списке сверху.' $script:ColWarn; return $null }
        $full += @('-s', $script:Serial)
    }
    $full += $ArgList
    if (-not $script:ServerUp) { Start-AdbServer }
    $argString = Join-Args $full
    if (-not $Quiet) { Write-Log ('> adb ' + $argString) $script:ColHead }
    $r = Invoke-Native $script:Adb $argString $Timeout
    if (-not $Quiet) {
        if ($r.Out) { Write-Log $r.Out }
        if ($r.Err) {
            $c = $script:ColDim
            if ($r.Code -ne 0) { $c = $script:ColErr }
            Write-Log $r.Err $c
        }
        if ($r.TimedOut) { Write-Log ("Команда не завершилась за {0} с и была прервана." -f $Timeout) $script:ColWarn }
        if ($r.Err -match 'unauthorized') {
            Write-Log 'Подсказка: разблокируйте планшет и разрешите отладку по USB в появившемся окне.' $script:ColWarn
        }
    }
    return $r
}

function Get-AdbDevices {
    $r = Invoke-Adb -ArgList @('devices', '-l') -NoDevice -Quiet -Timeout 15
    $list = @()
    if ($null -eq $r) { return $list }
    foreach ($line in ($r.Out -split "`r?`n")) {
        if ($line -match '^(\S+)\s+(device|unauthorized|offline|recovery|sideload|bootloader|authorizing|connecting|no permissions\S*)\s*(.*)$') {
            $serial = $Matches[1]; $state = $Matches[2]; $extra = $Matches[3]
            $model = ''
            if ($extra -match 'model:(\S+)') { $model = $Matches[1] -replace '_', ' ' }
            $list += [pscustomobject]@{ Serial = $serial; State = $state; Model = $model }
        }
    }
    return $list
}

function Get-StateText([string]$State) {
    switch ($State) {
        'device'       { return @('готово к работе', $script:ColOk) }
        'unauthorized' { return @('не разрешено на планшете: разблокируйте экран и нажмите «Разрешить»', $script:ColWarn) }
        'offline'      { return @('offline: переподключите кабель или перезапустите ADB-сервер', $script:ColErr) }
        'authorizing'  { return @('идёт авторизация, подождите', $script:ColWarn) }
        default        { return @($State, $script:ColWarn) }
    }
}

function Get-WindowsPortable {
    # Возвращает MTP/WPD-устройства Windows и то, что Проводник видит внутри них.
    $result = @()
    $wpd = @(Get-PnpDevice -PresentOnly -Class WPD -ErrorAction SilentlyContinue)
    $shell = New-Object -ComObject Shell.Application
    $pcItems = @($shell.NameSpace(17).Items())
    foreach ($d in $wpd) {
        $item = $pcItems | Where-Object { $_.Name -eq $d.FriendlyName } | Select-Object -First 1
        $names = @()
        if ($item) { $names = @($item.GetFolder.Items() | ForEach-Object { $_.Name }) }
        $result += [pscustomobject]@{
            Name = $d.FriendlyName; Status = $d.Status; Problem = "$($d.Problem)"
            InstanceId = $d.InstanceId; InExplorer = [bool]$item; Children = $names
        }
    }
    return $result
}

# ---------------------------------------------------------------- установка приложений

$script:Bundletool = $null

# Диалог выбора файлов. При отмене ничего не возвращает: вызывать как @(Select-Files ...) и проверять Count.
function Select-Files([string]$Title, [string]$Filter, [switch]$Multi) {
    $dlg = New-Object System.Windows.Forms.OpenFileDialog
    $dlg.Title = $Title
    $dlg.Filter = $Filter
    $dlg.Multiselect = [bool]$Multi
    if ($dlg.ShowDialog() -ne 'OK') { Write-Log 'Отменено.' $script:ColDim; return }
    return @($dlg.FileNames)
}

# Имя пакета из имени OBB: main.<версия>.<пакет>.obb или patch.<версия>.<пакет>.obb.
function Get-ObbPackage([string]$FileName) {
    $m = [regex]::Match([System.IO.Path]::GetFileName($FileName), '^(?:main|patch)\.\d+\.(?<pkg>[A-Za-z]\w*(?:\.\w+)+)\.obb$')
    if ($m.Success) { return $m.Groups['pkg'].Value }
    return $null
}

function Write-InstallHint([string]$Text) {
    if ($Text -match 'INSTALL_FAILED_UPDATE_INCOMPATIBLE|INSTALL_FAILED_SHARED_USER_INCOMPATIBLE') {
        Write-Log 'Подсказка: на планшете стоит это приложение с другой подписью. Удалите его (adb uninstall <пакет>) и повторите.' $script:ColWarn
    } elseif ($Text -match 'INSTALL_FAILED_VERSION_DOWNGRADE') {
        Write-Log 'Подсказка: на планшете версия новее. Удалите её (adb uninstall <пакет>) или ставьте более новую сборку.' $script:ColWarn
    } elseif ($Text -match 'INSTALL_FAILED_INSUFFICIENT_STORAGE') {
        Write-Log 'Подсказка: на планшете не хватает места.' $script:ColWarn
    } elseif ($Text -match 'INSTALL_FAILED_OLDER_SDK|INSTALL_FAILED_NO_MATCHING_ABIS|INSTALL_FAILED_DEPRECATED_SDK_VERSION') {
        Write-Log 'Подсказка: сборка не подходит этому планшету (версия Android или архитектура процессора).' $script:ColWarn
    } elseif ($Text -match 'USER_RESTRICTED|canceled by user|INSTALL_FAILED_ABORTED') {
        Write-Log 'Подсказка: установку блокирует планшет. Посмотрите на его экран и подтвердите установку; на Samsung отключите «Auto Blocker» в Настройки → Безопасность и конфиденциальность.' $script:ColWarn
    }
}

function Install-Apk([string]$Path) {
    $r = Invoke-Adb @('install', '-r', '-t', $Path) -Timeout 900
    if ($r -and $r.Code -eq 0 -and $r.Out -match 'Success') {
        Write-Log 'APK установлен.' $script:ColOk
        return $true
    }
    Write-Log 'Установить APK не удалось.' $script:ColErr
    if ($r) { Write-InstallHint "$($r.Out) $($r.Err)" }
    return $false
}

# Копирует OBB в /sdcard/Android/obb/<пакет>/. Пакет берётся из имени файла, иначе из $FallbackPackage.
function Send-Obb([string[]]$Files, [string]$FallbackPackage) {
    foreach ($f in $Files) {
        $pkg = Get-ObbPackage $f
        if (-not $pkg) { $pkg = $FallbackPackage }
        $name = [System.IO.Path]::GetFileName($f)
        $dir = "/sdcard/Android/obb/$pkg"
        $r = Invoke-Adb @('shell', 'mkdir', '-p', $dir)
        if (-not $r -or $r.Code -ne 0) { Write-Log 'Не удалось создать папку для OBB на планшете.' $script:ColErr; return $false }
        $r = Invoke-Adb @('push', $f, "$dir/$name") -Timeout 1800
        if (-not $r -or $r.Code -ne 0) { Write-Log "OBB не скопирован: $name" $script:ColErr; return $false }
    }
    Write-Log 'OBB скопирован.' $script:ColOk
    return $true
}

function Find-Java {
    if ($env:JAVA_HOME) {
        $p = Join-Path $env:JAVA_HOME 'bin\java.exe'
        if (Test-Path $p) { return $p }
    }
    $cmd = Get-Command java.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    return $null
}

function Find-Bundletool {
    foreach ($d in @($PSScriptRoot, (Join-Path $PSScriptRoot 'tools'), (Join-Path $PSScriptRoot 'platform-tools'))) {
        if (-not (Test-Path $d)) { continue }
        $f = Get-ChildItem -Path $d -Filter 'bundletool*.jar' -File -ErrorAction SilentlyContinue | Sort-Object Name -Descending | Select-Object -First 1
        if ($f) { return $f.FullName }
    }
    return $null
}

# Запускает внешнюю программу и пишет её вывод в журнал.
function Invoke-Logged([string]$File, [string[]]$ArgList, [int]$Timeout) {
    $argString = Join-Args $ArgList
    Write-Log "> $(Split-Path $File -Leaf) $argString" $script:ColHead
    $r = Invoke-Native $File $argString $Timeout
    if ($r.Out) { Write-Log $r.Out }
    if ($r.Err) {
        $c = $script:ColDim
        if ($r.Code -ne 0) { $c = $script:ColErr }
        Write-Log $r.Err $c
    }
    if ($r.TimedOut) { Write-Log ("Команда не завершилась за {0} с и была прервана." -f $Timeout) $script:ColWarn }
    return $r
}

# AAB ставится через bundletool: он собирает из бандла APK под выбранное устройство и устанавливает их.
function Install-Aab([string]$Aab) {
    if (-not $script:Adb) { Write-Log 'adb.exe не найден. Укажите путь к нему сверху.' $script:ColErr; return $false }
    if (-not $script:Serial) { Write-Log 'Сначала выберите устройство в списке сверху.' $script:ColWarn; return $false }
    $java = Find-Java
    if (-not $java) {
        Write-Log 'Java не найдена: bundletool работает на Java. Установите JRE или JDK 17+ (например, Temurin) и перезапустите утилиту.' $script:ColErr
        return $false
    }
    if (-not $script:Bundletool -or -not (Test-Path $script:Bundletool)) { $script:Bundletool = Find-Bundletool }
    if (-not $script:Bundletool) {
        Write-Log 'bundletool.jar не найден рядом с утилитой (его можно скачать на github.com/google/bundletool/releases). Укажите файл.' $script:ColWarn
        $f = @(Select-Files 'Укажите bundletool.jar' 'bundletool|*.jar|Все файлы|*.*')
        if ($f.Count -eq 0) { return $false }
        $script:Bundletool = $f[0]
    }
    if (-not (Test-Path (Join-Path $env:USERPROFILE '.android\debug.keystore'))) {
        Write-Log 'Не найден ~/.android/debug.keystore: bundletool не сможет подписать APK, и установка, скорее всего, не пройдёт.' $script:ColWarn
    }
    if (-not $script:ServerUp) { Start-AdbServer }

    $apks = Join-Path ([System.IO.Path]::GetTempPath()) ('adbtoolkit_{0}.apks' -f [guid]::NewGuid().ToString('N'))
    $common = @("--device-id=$($script:Serial)", "--adb=$($script:Adb)")
    try {
        $r = Invoke-Logged $java (@('-jar', $script:Bundletool, 'build-apks', "--bundle=$Aab", "--output=$apks", '--connected-device') + $common) 600
        if ($r.Code -ne 0) { Write-Log 'Не удалось собрать APK из AAB.' $script:ColErr; return $false }
        $r = Invoke-Logged $java (@('-jar', $script:Bundletool, 'install-apks', "--apks=$apks") + $common) 600
        if ($r.Code -eq 0) {
            Write-Log 'AAB установлен.' $script:ColOk
            return $true
        }
        Write-Log 'Установить AAB не удалось.' $script:ColErr
        Write-InstallHint "$($r.Out) $($r.Err)"
        return $false
    } finally {
        Remove-Item $apks -Force -ErrorAction SilentlyContinue
    }
}

# Имя пакета для OBB, у которых оно не читается из имени файла. '' - спрашивать не нужно, $null - пользователь отказался.
function Resolve-ObbPackage([string[]]$Files) {
    if (@($Files | Where-Object { -not (Get-ObbPackage $_) }).Count -eq 0) { return '' }
    $pkg = [Microsoft.VisualBasic.Interaction]::InputBox('Имя OBB не похоже на main.<версия>.<пакет>.obb. Введите имя пакета приложения:', 'Пакет для OBB', '')
    if (-not $pkg.Trim()) { Write-Log 'Отменено.' $script:ColDim; return $null }
    return $pkg.Trim()
}

# Разбирает перетащенные файлы по расширениям и решает, что с ними делать.
function Get-DropPlan([string[]]$Paths) {
    $plan = [pscustomobject]@{ Apks = @(); Obbs = @(); Aabs = @(); Ignored = @(); Error = $null }
    foreach ($p in $Paths) {
        $ext = [System.IO.Path]::GetExtension($p).ToLowerInvariant()
        if (Test-Path -LiteralPath $p -PathType Container) { $plan.Ignored += $p }
        elseif ($ext -eq '.apk') { $plan.Apks += $p }
        elseif ($ext -eq '.obb') { $plan.Obbs += $p }
        elseif ($ext -eq '.aab') { $plan.Aabs += $p }
        else { $plan.Ignored += $p }
    }
    if ($plan.Aabs.Count -gt 1) {
        $plan.Error = 'Перетащено несколько AAB: ставьте их по одному.'
    } elseif ($plan.Aabs.Count -gt 0 -and $plan.Apks.Count -gt 0) {
        $plan.Error = 'AAB и APK вместе: непонятно, что ставить. Перетащите что-то одно (OBB можно добавить к любому).'
    } elseif ($plan.Aabs.Count + $plan.Apks.Count + $plan.Obbs.Count -eq 0) {
        $plan.Error = 'Среди файлов нет APK, OBB или AAB.'
    }
    return $plan
}

# AAB или APK ставятся, OBB копируются после установки. Только OBB: приложение уже должно стоять на планшете.
function Install-Dropped([string[]]$Paths) {
    $plan = Get-DropPlan $Paths
    foreach ($i in $plan.Ignored) { Write-Log "Пропущено (не APK, OBB и не AAB): $i" $script:ColWarn }
    if ($plan.Error) { Write-Log $plan.Error $script:ColErr; return }
    $fallback = ''
    if ($plan.Obbs.Count -gt 0) {
        $fallback = Resolve-ObbPackage $plan.Obbs
        if ($null -eq $fallback) { return }
    }
    foreach ($f in $plan.Aabs) {
        Write-Log "AAB: $([System.IO.Path]::GetFileName($f))" $script:ColHead
        if (-not (Install-Aab $f)) { return }
    }
    foreach ($f in $plan.Apks) {
        Write-Log "APK: $([System.IO.Path]::GetFileName($f))" $script:ColHead
        if (-not (Install-Apk $f)) { return }
    }
    if ($plan.Obbs.Count -gt 0) {
        if ($plan.Aabs.Count + $plan.Apks.Count -eq 0) { Write-Log 'APK не передан: копирую только OBB, приложение должно уже стоять на планшете.' $script:ColWarn }
        $null = Send-Obb $plan.Obbs $fallback
    }
}

# ---------------------------------------------------------------- команды

$script:Commands = @(
    # ---------------- Авто
    @{ Group = 'Быстрое решение'
       Title = 'Авто-диагностика'
       Desc  = 'Проверяет цепочку: ADB, разрешение на планшете, USB-режим, блокировку экрана, вид из Проводника. Если режим не MTP, включает его.'
       Cmd   = 'несколько проверок + svc usb setFunctions mtp'
       Run   = {
           Update-DeviceList -Force
           $dev = $script:Devices | Where-Object { $_.Serial -eq $script:Serial } | Select-Object -First 1
           if (-not $dev) {
               Write-Log '[ADB] Устройств не видно. Проверьте кабель (нужен data-кабель) и порт; на планшете должна быть включена отладка по USB.' $script:ColErr
           } elseif ($dev.State -eq 'unauthorized') {
               Write-Log '[ADB] Планшет не дал разрешение. Разблокируйте экран и нажмите «Разрешить» в окне отладки по USB.' $script:ColErr
           } elseif ($dev.State -ne 'device') {
               Write-Log ("[ADB] Состояние: {0}. Попробуйте «Перезапустить ADB-сервер» и переподключить кабель." -f $dev.State) $script:ColErr
           } else {
               Write-Log '[ADB] Устройство отвечает.' $script:ColOk
               $lock = Invoke-Adb @('shell', "dumpsys window | grep -E 'isKeyguardShowing'") -Quiet
               if ($lock.Out -match 'isKeyguardShowing=true') {
                   Write-Log '[Экран] Планшет заблокирован: пока он заблокирован, Android не отдаёт файлы. Разблокируйте его.' $script:ColErr
               } else {
                   Write-Log '[Экран] Планшет разблокирован.' $script:ColOk
               }
               $usb = Invoke-Adb @('shell', 'getprop', 'sys.usb.config') -Quiet
               Write-Log ("[USB] Текущий режим: {0}" -f $usb.Out)
               if ($usb.Out -notmatch 'mtp') {
                   Write-Log '[USB] MTP не включён, включаю.' $script:ColWarn
                   $null = Invoke-Adb @('shell', 'svc', 'usb', 'setFunctions', 'mtp')
                   Start-Sleep -Seconds 4
                   $usb = Invoke-Adb @('shell', 'getprop', 'sys.usb.config') -Quiet
                   Write-Log ("[USB] Режим после переключения: {0}" -f $usb.Out)
               } else {
                   Write-Log '[USB] MTP включён.' $script:ColOk
               }
           }
           $w = @(Get-WindowsPortable)
           if ($w.Count -eq 0) {
               Write-Log '[Windows] MTP-устройств не найдено.' $script:ColErr
           }
           foreach ($d in $w) {
               if ($d.Status -ne 'OK') {
                   Write-Log ("[Windows] {0}: статус {1}, проблема {2}" -f $d.Name, $d.Status, $d.Problem) $script:ColErr
               } elseif ($d.Children.Count -gt 0) {
                   Write-Log ("[Windows] {0}: Проводник видит {1}" -f $d.Name, ($d.Children -join ', ')) $script:ColOk
               } else {
                   Write-Log ("[Windows] {0}: драйвер в порядке, но внутри пусто. Обновите окно Проводника (F5) или нажмите «Перезапустить драйвер MTP»." -f $d.Name) $script:ColWarn
               }
           }
       } }

    # ---------------- Подключение
    @{ Group = 'Подключение ADB'
       Title = 'Список устройств'
       Desc  = 'Показывает устройства и их статус: device (всё хорошо), unauthorized (нет разрешения на планшете), offline (потеряна связь).'
       Cmd   = 'adb devices -l'
       Run   = { $null = Invoke-Adb @('devices', '-l') -NoDevice } }
    @{ Group = 'Подключение ADB'
       Title = 'Перезапустить ADB-сервер'
       Desc  = 'Лечит зависшее подключение и статус offline: останавливает и заново запускает фоновый сервис adb на компьютере.'
       Cmd   = 'adb kill-server && adb start-server'
       Run   = {
           $null = Invoke-Adb @('kill-server') -NoDevice
           $script:ServerUp = $false
           Write-Log '> adb start-server' $script:ColHead
           Start-AdbServer
           Write-Log 'ADB-сервер запущен.' $script:ColOk
       } }
    @{ Group = 'Подключение ADB'
       Title = 'Переподключить устройство'
       Desc  = 'Заставляет планшет заново согласовать связь с ADB. Помогает, если статус offline или окно разрешения не появилось.'
       Cmd   = 'adb -s <серийник> reconnect'
       Run   = { $null = Invoke-Adb @('reconnect') } }
    @{ Group = 'Подключение ADB'
       Title = 'Ждать устройство (30 с)'
       Desc  = 'Ждёт, пока планшет станет доступен. Удобно запустить и в это время нажать «Разрешить» на экране.'
       Cmd   = 'adb -s <серийник> wait-for-device'
       Run   = { $null = Invoke-Adb @('wait-for-device') -Timeout 30 } }

    # ---------------- USB-режим
    @{ Group = 'Режим USB на планшете'
       Title = 'Текущий режим USB'
       Desc  = 'Показывает, какие USB-функции включены. Если в списке есть mtp, передача файлов работает; sec_charging значит только зарядка.'
       Cmd   = 'adb shell getprop sys.usb.config'
       Run   = {
           $null = Invoke-Adb @('shell', 'getprop', 'sys.usb.config')
           $null = Invoke-Adb @('shell', 'getprop', 'sys.usb.state')
       } }
    @{ Group = 'Режим USB на планшете'
       Title = 'Передача файлов (MTP)'
       Desc  = 'Включает режим передачи файлов, даже если выбор в шторке не сработал. Именно это открыло файлы планшета в Проводнике.'
       Cmd   = 'adb shell svc usb setFunctions mtp'
       Run   = {
           $null = Invoke-Adb @('shell', 'svc', 'usb', 'setFunctions', 'mtp')
           Start-Sleep -Seconds 3
           $null = Invoke-Adb @('shell', 'getprop', 'sys.usb.config')
       } }
    @{ Group = 'Режим USB на планшете'
       Title = 'Фото (PTP)'
       Desc  = 'Режим передачи изображений: планшет виден как камера. Нужен редко, например для старых программ импорта фото.'
       Cmd   = 'adb shell svc usb setFunctions ptp'
       Run   = { $null = Invoke-Adb @('shell', 'svc', 'usb', 'setFunctions', 'ptp') } }
    @{ Group = 'Режим USB на планшете'
       Title = 'USB-модем (RNDIS)'
       Desc  = 'Раздаёт интернет планшета на компьютер по кабелю. Передача файлов при этом отключается.'
       Cmd   = 'adb shell svc usb setFunctions rndis'
       Run   = { $null = Invoke-Adb @('shell', 'svc', 'usb', 'setFunctions', 'rndis') } }
    @{ Group = 'Режим USB на планшете'
       Title = 'Только зарядка'
       Desc  = 'Отключает передачу данных, оставляет зарядку.'
       Cmd   = 'adb shell svc usb setFunctions'
       Run   = { $null = Invoke-Adb @('shell', 'svc', 'usb', 'setFunctions') } }
    @{ Group = 'Режим USB на планшете'
       Title = 'MTP при разблокировке'
       Desc  = 'Просит Android включать передачу файлов каждый раз, когда вы разблокируете экран с подключённым кабелем.'
       Cmd   = 'adb shell svc usb setScreenUnlockedFunctions mtp'
       Run   = { $null = Invoke-Adb @('shell', 'svc', 'usb', 'setScreenUnlockedFunctions', 'mtp') } }

    # ---------------- Экран
    @{ Group = 'Экран и питание'
       Title = 'Разбудить экран'
       Desc  = 'Включает погасший экран. Это не снимает блокировку: PIN или рисунок всё равно вводятся на планшете.'
       Cmd   = 'adb shell input keyevent KEYCODE_WAKEUP'
       Run   = { $null = Invoke-Adb @('shell', 'input', 'keyevent', 'KEYCODE_WAKEUP') } }
    @{ Group = 'Экран и питание'
       Title = 'Проверить блокировку'
       Desc  = 'Показывает, заблокирован ли планшет. При isKeyguardShowing=true файлы по USB недоступны.'
       Cmd   = "adb shell dumpsys window | grep isKeyguardShowing"
       Run   = {
           $null = Invoke-Adb @('shell', "dumpsys window | grep -E 'isKeyguardShowing|mDreamingLockscreen'")
           $null = Invoke-Adb @('shell', "dumpsys power | grep mWakefulness=")
       } }
    @{ Group = 'Экран и питание'
       Title = 'Не гасить экран при USB'
       Desc  = 'Пока кабель подключён, экран не будет гаснуть и блокироваться по таймеру. Удобно при долгом копировании.'
       Cmd   = 'adb shell svc power stayon usb'
       Run   = { $null = Invoke-Adb @('shell', 'svc', 'power', 'stayon', 'usb') } }
    @{ Group = 'Экран и питание'
       Title = 'Вернуть автоотключение'
       Desc  = 'Отменяет предыдущую команду: экран снова гаснет по обычному таймеру.'
       Cmd   = 'adb shell svc power stayon false'
       Run   = { $null = Invoke-Adb @('shell', 'svc', 'power', 'stayon', 'false') } }
    @{ Group = 'Экран и питание'
       Title = 'Перезагрузить планшет'
       Desc  = 'Обычная перезагрузка. Помогает, когда ничего другое не лечит USB.'
       Cmd   = 'adb reboot'
       Confirm = 'Перезагрузить выбранное устройство?'
       Run   = { $null = Invoke-Adb @('reboot') } }

    # ---------------- Диагностика
    @{ Group = 'Информация о планшете'
       Title = 'Сведения об устройстве'
       Desc  = 'Модель, версия Android, серийный номер, заряд батареи.'
       Cmd   = 'adb shell getprop ro.product.model / ro.build.version.release; dumpsys battery'
       Run   = {
           foreach ($p in 'ro.product.manufacturer', 'ro.product.model', 'ro.build.version.release', 'ro.serialno') {
               $r = Invoke-Adb @('shell', 'getprop', $p) -Quiet
               Write-Log ("{0} = {1}" -f $p, $r.Out)
           }
           $b = Invoke-Adb @('shell', "dumpsys battery | grep -E '^  (level|status|AC powered|USB powered):'") -Quiet
           Write-Log $b.Out
       } }
    @{ Group = 'Информация о планшете'
       Title = 'Свободное место'
       Desc  = 'Сколько занято и свободно во внутренней памяти.'
       Cmd   = 'adb shell df -h /sdcard'
       Run   = { $null = Invoke-Adb @('shell', 'df', '-h', '/sdcard') } }
    @{ Group = 'Информация о планшете'
       Title = 'Содержимое /sdcard'
       Desc  = 'Список папок внутренней памяти напрямую через ADB, в обход Проводника. Если здесь файлы есть, а в Проводнике пусто, проблема в MTP.'
       Cmd   = 'adb shell ls -l /sdcard'
       Run   = { $null = Invoke-Adb @('shell', 'ls', '-l', '/sdcard') } }

    # ---------------- Windows
    @{ Group = 'Сторона Windows'
       Title = 'Что видит Проводник'
       Desc  = 'Показывает MTP-устройства Windows, их статус драйвера и содержимое, которое видит Проводник. Пусто значит, что доступ не выдан.'
       Cmd   = 'Get-PnpDevice -Class WPD + Shell.Application'
       Run   = {
           $w = @(Get-WindowsPortable)
           if ($w.Count -eq 0) { Write-Log 'MTP-устройств в Windows не найдено.' $script:ColWarn }
           foreach ($d in $w) {
               Write-Log ("{0}: статус {1}, проблема {2}, в Проводнике: {3}" -f $d.Name, $d.Status, $d.Problem, $d.InExplorer)
               if ($d.Children.Count -eq 0) { Write-Log '  (внутри пусто)' $script:ColWarn }
               foreach ($c in $d.Children) { Write-Log ("  {0}" -f $c) $script:ColOk }
           }
       } }
    @{ Group = 'Сторона Windows'
       Title = 'USB-устройства Android'
       Desc  = 'Список подключённых USB-устройств и их драйверов. Ищите строки с кодом проблемы, отличным от CM_PROB_NONE.'
       Cmd   = 'Get-PnpDevice -PresentOnly -Class WPD, USB, AndroidUsbDeviceClass'
       Run   = {
           $devs = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue |
               Where-Object { $_.Class -in 'WPD', 'AndroidUsbDeviceClass' -or ($_.Class -eq 'USB' -and $_.FriendlyName -match 'Composite|Android|MTP|SAMSUNG|Mobile') }
           if (-not $devs) { Write-Log 'Подходящих устройств нет.' $script:ColWarn }
           foreach ($d in $devs) {
               $c = $script:ColOk
               if ($d.Status -ne 'OK') { $c = $script:ColErr }
               Write-Log ("{0,-8} {1,-22} {2} ({3})" -f $d.Status, $d.Class, $d.FriendlyName, $d.Problem) $c
           }
       } }
    @{ Group = 'Сторона Windows'
       Title = 'Перезапустить драйвер MTP'
       Desc  = 'Отключает и заново включает MTP-устройство в Windows. Потребуется подтвердить запрос администратора (UAC).'
       Cmd   = 'pnputil /restart-device <InstanceId>   (от администратора)'
       Confirm = 'Windows запросит права администратора. Продолжить?'
       Run   = {
           $w = @(Get-WindowsPortable)
           if ($w.Count -eq 0) { Write-Log 'MTP-устройств для перезапуска нет.' $script:ColWarn; return }
           $lines = ($w | ForEach-Object { "pnputil /restart-device '{0}'" -f $_.InstanceId }) -join '; '
           Write-Log ('> ' + $lines) $script:ColHead
           try {
               Start-Process powershell.exe -Verb RunAs -WindowStyle Hidden -Wait -ArgumentList @('-NoProfile', '-Command', $lines)
               Write-Log 'Драйвер перезапущен. Подождите пару секунд и нажмите F5 в Проводнике.' $script:ColOk
           } catch {
               Write-Log 'Запрос администратора отклонён или не удался.' $script:ColErr
           }
       } }
    @{ Group = 'Сторона Windows'
       Title = 'Открыть «Этот компьютер»'
       Desc  = 'Открывает Проводник на списке дисков и устройств, где виден планшет.'
       Cmd   = 'explorer shell:MyComputerFolder'
       Run   = { Start-Process explorer.exe 'shell:MyComputerFolder' } }

    # ---------------- Файлы
    @{ Group = 'Файлы через ADB'
       Title = 'Скачать с планшета…'
       Desc  = 'Копирует файл или папку с планшета на компьютер. Работает и тогда, когда MTP в Проводнике капризничает.'
       Cmd   = 'adb pull <путь на планшете> <папка на ПК>'
       Run   = {
           $remote = [Microsoft.VisualBasic.Interaction]::InputBox('Путь на планшете (файл или папка):', 'Скачать с планшета', '/sdcard/DCIM')
           if (-not $remote) { Write-Log 'Отменено.' $script:ColDim; return }
           $dlg = New-Object System.Windows.Forms.FolderBrowserDialog
           $dlg.Description = 'Куда сохранить'
           if ($dlg.ShowDialog() -ne 'OK') { Write-Log 'Отменено.' $script:ColDim; return }
           $null = Invoke-Adb @('pull', $remote.Trim(), $dlg.SelectedPath) -Timeout 900
       } }
    @{ Group = 'Файлы через ADB'
       Title = 'Отправить на планшет…'
       Desc  = 'Копирует выбранный файл с компьютера в папку Download на планшете.'
       Cmd   = 'adb push <файл> /sdcard/Download/'
       Run   = {
           $dlg = New-Object System.Windows.Forms.OpenFileDialog
           $dlg.Title = 'Файл для отправки на планшет'
           if ($dlg.ShowDialog() -ne 'OK') { Write-Log 'Отменено.' $script:ColDim; return }
           $null = Invoke-Adb @('push', $dlg.FileName, '/sdcard/Download/') -Timeout 900
       } }

    # ---------------- Установка приложений
    @{ Group = 'Установка приложений'
       Title = 'Установить APK…'
       Desc  = 'Устанавливает выбранный APK на планшет. Если приложение уже есть, обновляет его с сохранением данных.'
       Cmd   = 'adb install -r -t <файл.apk>'
       Run   = {
           $apk = @(Select-Files 'APK для установки' 'APK|*.apk|Все файлы|*.*')
           if ($apk.Count -eq 0) { return }
           $null = Install-Apk $apk[0]
       } }
    @{ Group = 'Установка приложений'
       Title = 'Установить APK + OBB…'
       Desc  = 'Ставит APK и копирует файлы OBB (кэш игры) в /sdcard/Android/obb/<пакет>/. Пакет берётся из имени OBB: main.<версия>.<пакет>.obb.'
       Cmd   = 'adb install -r -t <apk>; adb push <obb> /sdcard/Android/obb/<пакет>/'
       Run   = {
           $apk = @(Select-Files 'APK для установки' 'APK|*.apk|Все файлы|*.*')
           if ($apk.Count -eq 0) { return }
           $obb = @(Select-Files 'OBB-файлы (можно несколько)' 'OBB|*.obb|Все файлы|*.*' -Multi)
           if ($obb.Count -eq 0) { return }
           $fallback = Resolve-ObbPackage $obb
           if ($null -eq $fallback) { return }
           if (Install-Apk $apk[0]) { $null = Send-Obb $obb $fallback }
       } }
    @{ Group = 'Установка приложений'
       Title = 'Установить AAB…'
       Desc  = 'Собирает из AAB APK под этот планшет и ставит их через bundletool. Нужны Java и bundletool.jar (положите рядом с утилитой или укажите при запросе).'
       Cmd   = 'bundletool build-apks --connected-device; bundletool install-apks'
       Run   = {
           $aab = @(Select-Files 'AAB для установки' 'AAB|*.aab|Все файлы|*.*')
           if ($aab.Count -eq 0) { return }
           $null = Install-Aab $aab[0]
       } }
)

# ---------------------------------------------------------------- автотест без окна

if ($SelfTest) {
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    $obbCases = [ordered]@{
        'main.123.com.foo.bar.obb' = 'com.foo.bar'
        'patch.5.com.x_y.z1.obb'   = 'com.x_y.z1'
        'C:\dl\main.7.org.a.b.obb' = 'org.a.b'
        'game.obb'                 = $null
        'main.1.nopkg.obb'         = $null
    }
    foreach ($k in $obbCases.Keys) {
        $got = Get-ObbPackage $k
        $ok = ($got -eq $obbCases[$k])
        Write-Host ("Get-ObbPackage '{0}' -> '{1}': {2}" -f $k, $got, $(if ($ok) { 'OK' } else { 'FAIL' }))
    }
    $dropCases = @(
        @{ Paths = @('a.apk', 'b.OBB');        Apks = 1; Obbs = 1; Aabs = 0; Err = $false }
        @{ Paths = @('x.aab', 'main.1.a.b.obb'); Apks = 0; Obbs = 1; Aabs = 1; Err = $false }
        @{ Paths = @('only.obb');              Apks = 0; Obbs = 1; Aabs = 0; Err = $false }
        @{ Paths = @('x.aab', 'y.apk');        Apks = 1; Obbs = 0; Aabs = 1; Err = $true }
        @{ Paths = @('x.aab', 'y.aab');        Apks = 0; Obbs = 0; Aabs = 2; Err = $true }
        @{ Paths = @('notes.txt');             Apks = 0; Obbs = 0; Aabs = 0; Err = $true }
    )
    foreach ($dc in $dropCases) {
        $pl = Get-DropPlan $dc.Paths
        $ok = ($pl.Apks.Count -eq $dc.Apks -and $pl.Obbs.Count -eq $dc.Obbs -and $pl.Aabs.Count -eq $dc.Aabs -and ([bool]$pl.Error) -eq $dc.Err)
        Write-Host ("Get-DropPlan {0}: {1}" -f ($dc.Paths -join ', '), $(if ($ok) { 'OK' } else { 'FAIL' }))
    }
    Write-Host ("java: {0}; bundletool: {1}" -f (Find-Java), (Find-Bundletool))
    $script:Adb = Find-Adb
    Write-Host "adb: $script:Adb"
    $script:Devices = @(Get-AdbDevices)
    $script:Devices | Format-Table -AutoSize | Out-String | Write-Host
    if ($script:Devices.Count -gt 0) { $script:Serial = $script:Devices[0].Serial }
    function Update-DeviceList { param([switch]$Force) $script:Devices = @(Get-AdbDevices) }
    foreach ($title in 'Авто-диагностика', 'Текущий режим USB', 'Проверить блокировку', 'Сведения об устройстве', 'Что видит Проводник', 'USB-устройства Android', 'Перезапустить ADB-сервер') {
        $c = $script:Commands | Where-Object { $_.Title -eq $title }
        Write-Host "=== $title ==="
        & $c.Run
    }
    return
}

# ---------------------------------------------------------------- окно

Add-Type -TypeDefinition 'using System.Runtime.InteropServices; public class DpiAware { [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); }'
[void][DpiAware]::SetProcessDPIAware()
[System.Windows.Forms.Application]::EnableVisualStyles()

$form = New-Object System.Windows.Forms.Form
$form.Text = 'ADB Toolkit'
$form.StartPosition = 'CenterScreen'
$form.AutoScaleDimensions = New-Object System.Drawing.SizeF(96, 96)
$form.AutoScaleMode = 'Dpi'
$form.ClientSize = New-Object System.Drawing.Size(1180, 760)
$form.MinimumSize = New-Object System.Drawing.Size(900, 500)
$form.Font = New-Object System.Drawing.Font('Segoe UI', 9)

# --- верхняя панель
$top = New-Object System.Windows.Forms.Panel
$top.Dock = 'Top'; $top.Height = 74
$top.Padding = New-Object System.Windows.Forms.Padding(8)

function New-Label($text, $x, $y, $w) {
    $l = New-Object System.Windows.Forms.Label
    $l.Text = $text; $l.Location = New-Object System.Drawing.Point($x, $y)
    $l.Size = New-Object System.Drawing.Size($w, 22); $l.TextAlign = 'MiddleLeft'
    return $l
}

$lblAdb = New-Label 'adb.exe:' 8 8 80
$txtAdb = New-Object System.Windows.Forms.TextBox
$txtAdb.Location = New-Object System.Drawing.Point(92, 9); $txtAdb.Width = 560
$txtAdb.Anchor = 'Top,Left,Right'
$btnBrowse = New-Object System.Windows.Forms.Button
$btnBrowse.Text = 'Обзор…'; $btnBrowse.Location = New-Object System.Drawing.Point(660, 7); $btnBrowse.Size = New-Object System.Drawing.Size(90, 26)
$btnBrowse.Anchor = 'Top,Right'

$lblDev = New-Label 'Устройство:' 8 40 80
$cmbDev = New-Object System.Windows.Forms.ComboBox
$cmbDev.DropDownStyle = 'DropDownList'
$cmbDev.Location = New-Object System.Drawing.Point(92, 41); $cmbDev.Width = 380
$btnRefresh = New-Object System.Windows.Forms.Button
$btnRefresh.Text = 'Обновить'; $btnRefresh.Location = New-Object System.Drawing.Point(480, 39); $btnRefresh.Size = New-Object System.Drawing.Size(90, 26)
$chkAuto = New-Object System.Windows.Forms.CheckBox
$chkAuto.Text = 'Автообновление'; $chkAuto.Checked = $true
$chkAuto.Location = New-Object System.Drawing.Point(580, 41); $chkAuto.Size = New-Object System.Drawing.Size(140, 24)
$lblState = New-Label '' 730 40 440
$lblState.Anchor = 'Top,Left,Right'
$lblState.Font = New-Object System.Drawing.Font('Segoe UI', 9, [System.Drawing.FontStyle]::Bold)

$top.Controls.AddRange(@($lblAdb, $txtAdb, $btnBrowse, $lblDev, $cmbDev, $btnRefresh, $chkAuto, $lblState))

# --- нижняя панель: произвольная команда
$bottom = New-Object System.Windows.Forms.Panel
$bottom.Dock = 'Bottom'; $bottom.Height = 42
$lblCustom = New-Label 'adb' 8 9 36
$lblCustom.Font = New-Object System.Drawing.Font('Consolas', 10, [System.Drawing.FontStyle]::Bold)
$txtCustom = New-Object System.Windows.Forms.TextBox
$txtCustom.Location = New-Object System.Drawing.Point(46, 10); $txtCustom.Width = 900
$txtCustom.Anchor = 'Top,Left,Right'
$txtCustom.Font = New-Object System.Drawing.Font('Consolas', 10)
$btnCustom = New-Object System.Windows.Forms.Button
$btnCustom.Text = 'Выполнить'; $btnCustom.Location = New-Object System.Drawing.Point(956, 8); $btnCustom.Size = New-Object System.Drawing.Size(100, 28)
$btnCustom.Anchor = 'Top,Right'
$lblHint = New-Label 'Своя команда к выбранному устройству, например: shell getprop ro.product.model' 0 0 0
$lblHint.Visible = $false
$bottom.Controls.AddRange(@($lblCustom, $txtCustom, $btnCustom))

# --- центр: команды слева, вывод справа
$split = New-Object System.Windows.Forms.SplitContainer
$split.Dock = 'Fill'
$split.SplitterDistance = 640
$split.FixedPanel = 'Panel1'

$cmdPanel = New-Object System.Windows.Forms.Panel
$cmdPanel.Dock = 'Fill'; $cmdPanel.AutoScroll = $true
$split.Panel1.Controls.Add($cmdPanel)

# область, куда перетаскивают APK / OBB / AAB (размещается в разделе «Установка приложений», см. цикл строк команд)
$script:ZoneColor = [System.Drawing.Color]::FromArgb(244, 248, 253)
$script:ZoneHot   = [System.Drawing.Color]::FromArgb(205, 228, 255)
$zone = New-Object System.Windows.Forms.Label
$zone.AllowDrop = $true
$zone.Text = "Перетащите сюда APK, OBB или AAB: утилита сама поймёт, как их установить`r`n(или нажмите и выберите файлы)"
$zone.TextAlign = 'MiddleCenter'
$zone.BorderStyle = 'FixedSingle'
$zone.BackColor = $script:ZoneColor
$zone.ForeColor = [System.Drawing.Color]::FromArgb(30, 60, 110)
$zone.Font = New-Object System.Drawing.Font('Segoe UI', 9.5)
$zone.Cursor = [System.Windows.Forms.Cursors]::Hand

$outHeader = New-Object System.Windows.Forms.Panel
$outHeader.Dock = 'Top'; $outHeader.Height = 34
$btnClear = New-Object System.Windows.Forms.Button
$btnClear.Text = 'Очистить'; $btnClear.Location = New-Object System.Drawing.Point(6, 4); $btnClear.Size = New-Object System.Drawing.Size(90, 26)
$btnCopy = New-Object System.Windows.Forms.Button
$btnCopy.Text = 'Копировать всё'; $btnCopy.Location = New-Object System.Drawing.Point(102, 4); $btnCopy.Size = New-Object System.Drawing.Size(120, 26)
$outHeader.Controls.AddRange(@($btnClear, $btnCopy))

$rtb = New-Object System.Windows.Forms.RichTextBox
$rtb.Dock = 'Fill'; $rtb.ReadOnly = $true
$rtb.BackColor = [System.Drawing.Color]::FromArgb(30, 30, 30)
$rtb.ForeColor = $script:ColText
$rtb.Font = New-Object System.Drawing.Font('Consolas', 9.5)
$rtb.BorderStyle = 'None'; $rtb.WordWrap = $true
$script:Out = $rtb
$split.Panel2.Controls.Add($rtb)
$split.Panel2.Controls.Add($outHeader)

$form.Controls.Add($split)
$form.Controls.Add($bottom)
$form.Controls.Add($top)
$split.BringToFront()

# --- строки команд
$y = 6
$lastGroup = $null
$rowWidth = 600
foreach ($c in $script:Commands) {
    if ($c.Group -ne $lastGroup) {
        $h = New-Object System.Windows.Forms.Label
        $h.Text = '  ' + $c.Group
        $h.Font = New-Object System.Drawing.Font('Segoe UI', 9.5, [System.Drawing.FontStyle]::Bold)
        $h.BackColor = [System.Drawing.Color]::FromArgb(228, 236, 247)
        $h.ForeColor = [System.Drawing.Color]::FromArgb(30, 60, 110)
        $h.TextAlign = 'MiddleLeft'
        $h.Location = New-Object System.Drawing.Point(6, $y)
        $h.Size = New-Object System.Drawing.Size($rowWidth, 26)
        $h.Anchor = 'Top,Left,Right'
        $cmdPanel.Controls.Add($h)
        $y += 32
        $lastGroup = $c.Group
        if ($c.Group -eq 'Установка приложений') {
            # область перетаскивания лежит в начале этого раздела
            $zone.Location = New-Object System.Drawing.Point(6, $y)
            $zone.Size = New-Object System.Drawing.Size($rowWidth, 70)
            $zone.Anchor = 'Top,Left,Right'
            $cmdPanel.Controls.Add($zone)
            $y += 78
        }
    }
    $b = New-Object System.Windows.Forms.Button
    $b.Text = $c.Title
    $b.Tag = $c
    $b.Location = New-Object System.Drawing.Point(6, $y)
    $b.Size = New-Object System.Drawing.Size(190, 40)
    $cmdPanel.Controls.Add($b)

    $d = New-Object System.Windows.Forms.Label
    $d.Text = $c.Desc
    $d.Location = New-Object System.Drawing.Point(204, $y)
    $d.Size = New-Object System.Drawing.Size(($rowWidth - 200), 38)
    $d.Anchor = 'Top,Left,Right'
    $cmdPanel.Controls.Add($d)

    $k = New-Object System.Windows.Forms.Label
    $k.Text = $c.Cmd
    $k.Font = New-Object System.Drawing.Font('Consolas', 8)
    $k.ForeColor = [System.Drawing.Color]::FromArgb(120, 120, 120)
    $k.Location = New-Object System.Drawing.Point(204, ($y + 40))
    $k.Size = New-Object System.Drawing.Size(($rowWidth - 200), 16)
    $k.Anchor = 'Top,Left,Right'
    $k.AutoEllipsis = $true
    $cmdPanel.Controls.Add($k)

    $b.Add_Click({ param($s, $e) Invoke-Command-Button $s.Tag })
    $y += 64
}

# ---------------------------------------------------------------- логика окна

function Set-Busy([bool]$Value) {
    $script:Busy = $Value
    $cmdPanel.Enabled = -not $Value
    $btnCustom.Enabled = -not $Value
    $btnRefresh.Enabled = -not $Value
    if ($Value) { $form.Cursor = [System.Windows.Forms.Cursors]::WaitCursor } else { $form.Cursor = [System.Windows.Forms.Cursors]::Default }
}

function Update-StateLabel {
    $dev = $script:Devices | Where-Object { $_.Serial -eq $script:Serial } | Select-Object -First 1
    if (-not $script:Adb) {
        $lblState.Text = 'adb.exe не найден: укажите путь'; $lblState.ForeColor = $script:ColErr
    } elseif (-not $dev) {
        $lblState.Text = 'ADB не видит устройств'; $lblState.ForeColor = $script:ColErr
    } else {
        $t = Get-StateText $dev.State
        $lblState.Text = $t[0]
        $c = $t[1]
        # на светлом фоне формы берём более тёмные оттенки
        if ($c -eq $script:ColOk) { $c = [System.Drawing.Color]::FromArgb(20, 130, 40) }
        elseif ($c -eq $script:ColWarn) { $c = [System.Drawing.Color]::FromArgb(190, 110, 0) }
        elseif ($c -eq $script:ColErr) { $c = [System.Drawing.Color]::FromArgb(190, 30, 30) }
        $lblState.ForeColor = $c
    }
}

function Update-DeviceList {
    param([switch]$Force)
    if (-not $script:Adb) { Update-StateLabel; return }
    $list = @(Get-AdbDevices)
    $sig = ($list | ForEach-Object { "$($_.Serial)|$($_.State)" }) -join ';'
    if (-not $Force -and $sig -eq $script:DevSig) { return }
    $script:DevSig = $sig
    $script:Devices = $list
    $prev = $script:Serial
    $script:Updating = $true
    $cmbDev.Items.Clear()
    foreach ($d in $list) {
        $name = $d.Model
        if (-not $name) { $name = 'устройство' }
        [void]$cmbDev.Items.Add(("{0}  ({1})  [{2}]" -f $name, $d.Serial, $d.State))
    }
    $idx = -1
    for ($i = 0; $i -lt $list.Count; $i++) { if ($list[$i].Serial -eq $prev) { $idx = $i } }
    if ($idx -lt 0 -and $list.Count -gt 0) { $idx = 0 }
    if ($idx -ge 0) { $cmbDev.SelectedIndex = $idx; $script:Serial = $list[$idx].Serial } else { $script:Serial = $null }
    $script:Updating = $false
    Update-StateLabel
}

function Invoke-Command-Button($c) {
    if ($script:Busy) { return }
    if ($c.Confirm) {
        $ans = [System.Windows.Forms.MessageBox]::Show($c.Confirm, $c.Title, 'YesNo', 'Warning')
        if ($ans -ne 'Yes') { return }
    }
    Set-Busy $true
    try {
        Write-Log ('=== ' + $c.Title + ' ===') $script:ColHead
        & $c.Run
        Write-Log ''
    } catch {
        Write-Log ('Ошибка: ' + $_.Exception.Message) $script:ColErr
    } finally {
        Set-Busy $false
        Update-DeviceList
    }
}

function Invoke-Drop([string[]]$Paths) {
    if ($script:Busy) { Write-Log 'Дождитесь окончания текущей операции.' $script:ColWarn; return }
    Set-Busy $true
    try {
        Write-Log '=== Установка перетащенных файлов ===' $script:ColHead
        Install-Dropped $Paths
        Write-Log ''
    } catch {
        Write-Log ('Ошибка: ' + $_.Exception.Message) $script:ColErr
    } finally {
        Set-Busy $false
        Update-DeviceList
    }
}

function Split-CommandLine([string]$Line) {
    $tokens = @()
    foreach ($m in [regex]::Matches($Line, '"([^"]*)"|(\S+)')) {
        if ($m.Groups[1].Success) { $tokens += $m.Groups[1].Value } else { $tokens += $m.Groups[2].Value }
    }
    return $tokens
}

function Invoke-Custom {
    if ($script:Busy) { return }
    $line = $txtCustom.Text.Trim()
    if (-not $line) { return }
    $tokens = @(Split-CommandLine $line)
    $noDev = $tokens[0] -in 'devices', 'kill-server', 'start-server', 'version', 'help', 'connect', 'disconnect'
    Set-Busy $true
    try {
        if ($noDev) { $null = Invoke-Adb $tokens -NoDevice } else { $null = Invoke-Adb $tokens }
        Write-Log ''
    } finally {
        Set-Busy $false
        Update-DeviceList
    }
}

$cmbDev.Add_SelectedIndexChanged({
    if ($script:Updating) { return }
    $i = $cmbDev.SelectedIndex
    if ($i -ge 0 -and $i -lt $script:Devices.Count) { $script:Serial = $script:Devices[$i].Serial }
    Update-StateLabel
})
# Установку запускаем не из самого обработчика drop: пока он не вернулся, Проводник держит перетаскивание.
$script:DropFiles = @()
$dropTimer = New-Object System.Windows.Forms.Timer
$dropTimer.Interval = 50
$dropTimer.Add_Tick({
    $dropTimer.Stop()
    $files = @($script:DropFiles)
    $script:DropFiles = @()
    Invoke-Drop $files
})
$zone.Add_DragEnter({
    param($s, $e)
    if ($e.Data.GetDataPresent([System.Windows.Forms.DataFormats]::FileDrop)) {
        $e.Effect = [System.Windows.Forms.DragDropEffects]::Copy
        $zone.BackColor = $script:ZoneHot
    } else {
        $e.Effect = [System.Windows.Forms.DragDropEffects]::None
    }
})
$zone.Add_DragLeave({ $zone.BackColor = $script:ZoneColor })
$zone.Add_DragDrop({
    param($s, $e)
    $zone.BackColor = $script:ZoneColor
    $script:DropFiles = @($e.Data.GetData([System.Windows.Forms.DataFormats]::FileDrop))
    $dropTimer.Start()
})
$zone.Add_Click({
    if ($script:Busy) { return }
    $files = @(Select-Files 'APK, OBB или AAB' 'APK, OBB, AAB|*.apk;*.obb;*.aab|Все файлы|*.*' -Multi)
    if ($files.Count -gt 0) { Invoke-Drop $files }
})
$btnRefresh.Add_Click({ Update-DeviceList -Force })
$btnClear.Add_Click({ $rtb.Clear() })
$btnCopy.Add_Click({ if ($rtb.TextLength -gt 0) { [System.Windows.Forms.Clipboard]::SetText($rtb.Text) } })
$btnCustom.Add_Click({ Invoke-Custom })
$txtCustom.Add_KeyDown({ param($s, $e) if ($e.KeyCode -eq 'Return') { $e.SuppressKeyPress = $true; Invoke-Custom } })
$btnBrowse.Add_Click({
    $dlg = New-Object System.Windows.Forms.OpenFileDialog
    $dlg.Filter = 'adb.exe|adb.exe'
    if ($dlg.ShowDialog() -eq 'OK') {
        $script:Adb = $dlg.FileName; $txtAdb.Text = $dlg.FileName
        $script:ServerUp = $false
        Update-DeviceList -Force
    }
})

$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 3000
$timer.Add_Tick({
    if ($chkAuto.Checked -and -not $script:Busy -and $script:Adb) {
        $script:Busy = $true
        try { Update-DeviceList } finally { $script:Busy = $false }
    }
})

$form.Add_Shown({
    $script:Adb = Find-Adb
    if ($script:Adb) { $txtAdb.Text = $script:Adb }
    else { Write-Log 'adb.exe не найден. Укажите путь к нему сверху (папка platform-tools).' $script:ColErr }
    Write-Log 'Выберите устройство и нажмите нужную кнопку. Начните с «Авто-диагностики».' $script:ColDim
    Write-Log ''
    Update-DeviceList -Force
    $timer.Start()
})
$form.Add_FormClosed({ $timer.Stop() })

[void]$form.ShowDialog()
