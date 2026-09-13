# EcoPaste Windows 内存基准脚本（开发工具，非产品功能）。
# 场景：启动 -> 基线 -> 批量写入混合内容 -> 峰值 -> 隐藏静置 -> 稳态，产出 CSV。
# 用法：在 `pnpm tauri dev` 运行中的 dev 环境执行（dev 进程名为 eco_paste_lib.exe?：
# 开发二进制名固定为 EcoPaste.exe / eco_paste_lib 的宿主进程）。
#   .\scripts\mem-bench.ps1 [-ProcessName EcoPaste] [-Rounds 200] [-IdleMinutes 5]
# 产出：mem-bench-<timestamp>.csv（阶段,时间戳,RSS字节,虚拟内存字节）
# 依赖：clip.exe（Windows 自带）。图片轮次用 PowerShell 生成临时 PNG 再入剪贴板。

param(
    [string]$ProcessName = "EcoPaste",
    [int]$Rounds = 200,
    [int]$IdleMinutes = 5,
    [string]$CsvPath = "mem-bench-$(Get-Date -Format 'yyyyMMdd-HHmmss').csv"
)

$ErrorActionPreference = "Stop"

if ($Rounds -lt 1 -or $Rounds -gt 5000) {
    throw "Rounds 必须在 1..5000 之间"
}

function Get-MemRow {
    param([string]$Stage)
    $proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue |
        Sort-Object WorkingSet64 -Descending | Select-Object -First 1
    if ($null -eq $proc) {
        throw "找不到进程 $ProcessName；请先启动 EcoPaste（或用 -ProcessName 指定 dev 进程名）"
    }
    $proc.Refresh()
    [PSCustomObject]@{
        stage    = $Stage
        time     = (Get-Date -Format "o")
        rss      = $proc.WorkingSet64
        virtual  = $proc.PagedMemorySize64
    }
}

function Write-CsvRow {
    param([PSCustomObject]$Row)
    $line = "$($Row.stage),$($Row.time),$($Row.rss),$($Row.virtual)"
    Add-Content -Path $CsvPath -Value $line -Encoding utf8
    Write-Host ("{0,-12} RSS={1,10:N0}  VM={2,12:N0}" -f $Row.stage, $Row.rss, $Row.virtual)
}

# --- 准备 CSV 头 ---
@("stage,time,rss,virtual") | Set-Content -Path $CsvPath -Encoding utf8

Write-Host "内存基准：进程=$ProcessName 轮次=$Rounds 静置=$IdleMinutes 分钟"
Write-Host "CSV 产出：$CsvPath"

# --- 基线（启动后静置 10 秒再取，避开启动抖动）---
Start-Sleep -Seconds 10
Write-CsvRow (Get-MemRow "baseline")

# --- 批量复制：文本(短/长)、URL、JSON、图片 混合 ---
# 剪贴板写入失败（EcoPaste watcher 或其他监听者短暂持锁）按 watcher 同款
# 退避节奏重试；连续 5 次失败才放弃本轮。
function Write-ClipboardWithRetry {
    param(
        [scriptblock]$Write,
        [string]$Label
    )
    foreach ($delay in @(0, 50, 120, 250)) {
        if ($delay -gt 0) { Start-Sleep -Milliseconds $delay }
        try {
            & $Write
            return $true
        } catch {
            Write-Verbose "clipboard write ($Label) retry after ${delay}ms: $($_.Exception.Message)"
        }
    }
    Write-Warning "clipboard write ($Label) failed after retries; skip round"
    return $false
}

$pngPath = Join-Path $env:TEMP "ecopaste-mem-bench.png"
Add-Type -AssemblyName System.Drawing
$bmp = New-Object System.Drawing.Bitmap(1920, 1080)
$gfx = [System.Drawing.Graphics]::FromImage($bmp)
$gfx.Clear([System.Drawing.Color]::SteelBlue)
$bmp.Save($pngPath, [System.Drawing.Imaging.ImageFormat]::Png)
$gfx.Dispose(); $bmp.Dispose()

$payloads = @(
    "短文本基准样本 $([char]0x4E2D)$([char]0x6587)",
    ("长文本样本 " + ("0123456789abcdef" * 256)),
    "https://github.com/EcoPasteHub/EcoPaste",
    '{"kind":"bench","round":1,"tags":["memory","baseline"]}'
)

for ($i = 0; $i -lt $Rounds; $i++) {
    switch ($i % 5) {
        4 {
            # Set-Clipboard -Path 在部分环境（剪贴板被监听者持锁时）不可用，
            # 用 WinForms 位图入板替代。
            Write-ClipboardWithRetry -Label "image-$i" -Write {
                Add-Type -AssemblyName System.Windows.Forms
                $img = [System.Drawing.Image]::FromFile($pngPath)
                [System.Windows.Forms.Clipboard]::SetImage($img)
                $img.Dispose()
            } | Out-Null
        }
        default {
            $text = $payloads[$i % 4]
            Write-ClipboardWithRetry -Label "text-$i" -Write { $text | Set-Clipboard } | Out-Null
        }
    }
    # 给 watcher 的去抖与入库留出间隔，模拟真实复制节奏。
    Start-Sleep -Milliseconds 200

    if (($i + 1) % 50 -eq 0) {
        Write-CsvRow (Get-MemRow "copy-$($i + 1)")
    }
}

Write-CsvRow (Get-MemRow "peak")
Remove-Item $pngPath -ErrorAction SilentlyContinue

# --- 隐藏静置：等 watcher 空闲、列表窗口隐藏后的稳态 ---
Write-Host "静置 $IdleMinutes 分钟后取稳态……"
Start-Sleep -Seconds ($IdleMinutes * 60)
Write-CsvRow (Get-MemRow "steady")

Write-Host "完成。基准 CSV 已写入 $CsvPath（可与优化前后版本对比 rss 列）"
