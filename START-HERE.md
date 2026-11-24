# fntidy · Windows-x64-Vorschau

Entpacke das ZIP und öffne PowerShell im entpackten Ordner. Zum Ausprobieren
genügt die mitgelieferte `fntidy.exe`. Die Terminalausgabe ist auf Englisch.

## 1. Testordner erstellen und Vorschau prüfen

Das Beispiel erstellt einen Ordner mit Emoji und einer Datei darin. Damit
lässt sich auch die Vorschau nach einer Ordnerumbenennung nachvollziehen.

```powershell
$ordner = Join-Path $env:TEMP ("fntidy-test-" + [guid]::NewGuid())
$unterordner = Join-Path $ordner "Fotos 📷"
New-Item -ItemType Directory -Path $unterordner -Force | Out-Null
Set-Content -LiteralPath (Join-Path $unterordner "März  🇨🇭.txt") -Value "Testinhalt"

$plan = "$ordner.json"
.\fntidy.exe plan $ordner --recursive --profile tidy --out $plan
```

Die Vorschau zählt **eine Datei und einen Ordner**. Unter `Final path` steht
`Fotos\März.txt`, weil sich beide Namen ändern. Die Zusammenfassung nennt
auch die Anzahl blockierter Dateien und Ordner. Bei einem Konflikt prüfe
die genannten Pfade und erstelle nach der Korrektur einen neuen Plan mit
einem freien Dateinamen für die JSON-Datei.

`--recursive` bezieht alle Unterordner ein. `--profile tidy` entfernt
erkannte Emojis, vereinheitlicht Leerzeichen und wendet die Windows-Regeln
an. Für reine Windows-Namensreparaturen verwende `--profile windows`.
Mit `check` kannst du die Namensverstösse ohne Umbenennungsvorschläge anzeigen.

## 2. Den geprüften Plan anwenden

```powershell
.\fntidy.exe verify-plan $plan
```

Wenn die Prüfung erfolgreich ist und die Vorschläge stimmen, führe den Plan aus.

```powershell
.\fntidy.exe apply $plan
```

fntidy benennt zuerst die Datei, dann ihren Ordner um. In der
Zusammenfassung steht `Changed: 1 file, 1 folder.` Kopiere den vollständigen
Pfad aus der Zeile `Recovery journal:`. Das Protokoll liegt unter
`%LOCALAPPDATA%\fntidy\runs`. Eine neue Vorschau für denselben Ordner
ergibt jetzt null Vorschläge.

```powershell
.\fntidy.exe plan $ordner --recursive --profile tidy
```

## 3. Lauf prüfen und ursprüngliche Namen wiederherstellen

Setze `$protokoll` auf den kopierten Pfad zur `run.jsonl`.

```powershell
$protokoll = "C:\Pfad\zum\run.jsonl"
.\fntidy.exe recover $protokoll
.\fntidy.exe undo $protokoll
Get-Content -LiteralPath (Join-Path $unterordner "März  🇨🇭.txt")
```

`recover` zeigt den Status rein lesend an. `undo` stellt die Namen in
umgekehrter Reihenfolge wieder her. Die letzte Zeile liest aus der Datei
unter ihrem ursprünglichen Namen wieder `Testinhalt`.

Bei einem Konflikt stoppt fntidy und nennt die betroffenen Pfade. Der
bis dahin bestätigte Fortschritt steht im Protokoll. Nach einem
unterbrochenen Rückgängigmachen kannst du nach der Klärung denselben
`undo`-Befehl erneut ausführen.
