# fntidy · Windows-x64-Vorschau

Entpacke das ZIP und öffne PowerShell im entpackten Ordner. Rust ist zum
Ausprobieren nicht erforderlich. Diese Vorschau hat englische Terminalausgaben.

## Im Beispielordner ausprobieren

```powershell
.\fntidy.exe --version
$sample = Join-Path $env:TEMP ("fntidy-test-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $sample
Set-Content -LiteralPath (Join-Path $sample "Beispiel.txt") -Value "Test"
.\fntidy.exe check $sample
$plan = "$sample.json"
.\fntidy.exe plan $sample --out $plan
.\fntidy.exe verify-plan $plan
```

Der gültige Name ergibt keine Verstösse und keine vorgeschlagene Umbenennung.
Zum Prüfen eigener Namen verwende möglichst einen Ordner mit Kopien. Die
geprüften Dateien bleiben bei diesen Befehlen unverändert.

## Emojis und Leerzeichen bereinigen

```powershell
.\fntidy.exe check $sample --profile tidy
.\fntidy.exe plan $sample --profile tidy --out "$sample-tidy.json"
```

Mit `tidy` wird zum Beispiel aus `März  🇨🇭.tar.gz` der Name
`März.tar.gz`. Umlaute, gültige Satzzeichen und übliche Dateiendungen
bleiben erhalten. Bereits belegte Zielnamen blockieren den Plan.

Mit `--recursive` werden auch alle Unterordner geprüft. Der folgende
Befehl zeigt die vorgeschlagenen Umbenennungen relativ zum Testordner an.

```powershell
.\fntidy.exe plan $sample --recursive --profile tidy
```

## Einen Plan auf eine Testkopie anwenden

```powershell
$ordner = "C:\Pfad\zur\Testkopie"
$plan = Join-Path $env:TEMP "fntidy-$([guid]::NewGuid()).json"
.\fntidy.exe plan $ordner --recursive --profile tidy --out $plan
.\fntidy.exe verify-plan $plan
```

Prüfe die Vorschläge. Wenn der Plan bereit ist, wendest du ihn so an.

```powershell
.\fntidy.exe apply $plan
```

Dateien und Ordner werden von innen nach aussen umbenannt. Bei einem
Hindernis während des Laufs stoppt fntidy. Bereits bestätigte
Umbenennungen bleiben bestehen. Die Ausgabe nennt den Pfad zum Protokoll
unter `%LOCALAPPDATA%\fntidy\runs`. Automatisches Rückgängigmachen,
weitere Zielprofile und eine deutschsprachige Bedienung sind spätere Etappen.
