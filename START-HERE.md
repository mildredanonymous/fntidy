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

`apply <plandatei>` kann in dieser Vorschau genau eine vorgeschlagene
Dateiumbenennung anwenden. Verwende dafür nur Testkopien. Das Protokoll
liegt unter `%LOCALAPPDATA%\fntidy\runs`; die Ausgabe nennt den genauen Pfad.
Automatisches Rückgängigmachen, rekursive Verarbeitung, weitere Zielprofile
und eine deutschsprachige Bedienung folgen in späteren Etappen.
