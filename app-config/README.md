# app-config (base do futuro app Tauri)

UI estatica em `index.html`. Cada botao equivale a um comando do `cursor-ctl`:

- Aplicar: `cursor-ctl apply <arquivo.cur>`
- Restaurar: `cursor-ctl restore`
- Efeitos: `cursor-ctl effects on|off`
- Som: `cursor-ctl sound on|off [--volume V]`
- Ver config: `cursor-ctl show`

Proximo passo: criar o projeto Tauri e chamar esses comandos via `Command`
do backend, reaproveitando este HTML como webview.
