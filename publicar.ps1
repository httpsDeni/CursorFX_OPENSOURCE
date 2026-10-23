# Sobe o projeto para o GitHub (rode uma vez; pede login na hora do push)
$ErrorActionPreference = 'Stop'
$repo = 'https://github.com/httpsDeni/CursorFX_OPENSOURCE.git'
cd $PSScriptRoot
if (-not (Test-Path .git)) { git init }
git add -A
$st = git status --porcelain
if ($st) { git commit -m 'Cursor Open: app open source de cursor, efeitos e sons' }
git branch -M main
if (-not (git remote 2>$null | Select-String '^origin$')) { git remote add origin $repo }
git push -u origin main
echo 'Pronto: confira no navegador.'
