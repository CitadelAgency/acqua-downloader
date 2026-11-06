# dlm

Gerenciador de downloads para Windows em Rust, com janela Tauri/Svelte, bandeja, CLI e extensão Chromium. O motor faz downloads segmentados, retomada com journal e verificação antes de entregar o arquivo.

## Instalar

1. Baixe `dlm_2.0.0_x64-setup.exe` em [Releases](https://github.com/CitadelAgency/dlm/releases/latest) e execute. Não pede administrador e instala o WebView2 se faltar.
2. O instalador não tem assinatura digital, então o Windows pode mostrar "O Windows protegeu o computador". Clique em **Mais informações** e **Executar assim mesmo**.
3. Para capturar downloads do navegador, abra `chrome://extensions` (ou `edge://extensions`, `brave://extensions`), ligue o **Modo do desenvolvedor**, clique em **Carregar sem compactação** e escolha a pasta da extensão. O caminho é `%LOCALAPPDATA%\dlm\extension`; o menu Iniciar tem o atalho **dlm - pasta da extensão**.

Windows 10/11, 64 bits.

## Compilar

Requisitos: Rust MSVC 1.85+ (validado com 1.93), Visual Studio Build Tools com C++, Windows SDK, Node 20+ e WebView2.

```powershell
.\build.ps1 -Check
# build\bin\dlm-rust.exe
```

Compilação manual:

```powershell
npm --prefix frontend ci
npm --prefix frontend run build
cargo build --locked --release --features custom-protocol
# target\release\dlm.exe
```

A feature `custom-protocol` incorpora `frontend/dist`, sem servidor Vite. Refaça o build Rust depois de alterar o frontend. Somente CLI: `cargo build --release --no-default-features --bin dlm`.

Instalador (gera `target\release\bundle\nsis\dlm_2.0.0_x64-setup.exe`):

```powershell
npm --prefix frontend ci
npx @tauri-apps/cli@2 build --bundles nsis
```

## Executar

```powershell
.\build\bin\dlm-rust.exe
.\build\bin\dlm-rust.exe --tray
.\build\bin\dlm-rust.exe get https://servidor/arquivo.zip -o D:\Downloads -c 16
.\build\bin\dlm-rust.exe resume D:\Downloads\arquivo.zip.dlm
.\build\bin\dlm-rust.exe relink D:\Downloads\arquivo.zip.dlm https://servidor/link-novo
.\build\bin\dlm-rust.exe chrome list
.\build\bin\dlm-rust.exe help
```

`get` aceita `-name`, `-hash sha256:HEX|md5:HEX`, `-mirror URL` repetido, `-limit 5M`, `-H "Nome: valor"`, `-q` e `-v`. Códigos: 0 sucesso, 1 erro/pausa, 2 uso inválido, 3 link expirado, 4 integridade. Ctrl+C conserva o progresso confirmado.

A bandeja guarda a fila em `%LOCALAPPDATA%\dlm\dlm.db`. A janela é outro processo: fechá-la mantém os downloads. Há prioridade, limites de banda, agenda, idiomas, notificações, inicialização com Windows e observação opcional da área de transferência.

A barra de título é integrada ao cabeçalho, com controles próprios de minimizar, maximizar/restaurar e fechar. Arraste a marca ou o espaço livre do cabeçalho para mover a janela; dois cliques nessa área alternam a maximização. Fechar a janela mantém a fila na bandeja.

## Extensão

Carregue a pasta `extension` sem compactação no Chrome, Edge ou Brave. O ID é fixo: `djlnbnckkmhijbocpoehdlcfnkfnikbi`. A bandeja registra o host automaticamente. Também há `register-host` e `unregister-host`.

Native Messaging e named pipe com ACL do usuário fazem a comunicação, sem porta HTTP. A extensão envia URL, cookies, referrer e User-Agent. Somente a origem autorizada pode enviar `ping` e `add`.

## Integridade

- Intervalos exigem 206, Content-Range, tamanho e validador compatíveis. Um 200 em retomada é recusado.
- O parcial recebe `sync_data` antes de intervalos e hashes serem sincronizados no journal CRC32C.
- Blocos de 4 MiB usam xxh3. Retomadas verificam blocos, emendas de 64 KiB e amostras antes de gravar.
- A entrega exige releitura completa e SHA-256; hashes informados, publicados em cabeçalhos ou em `.sha256` precisam conferir.
- O arquivo recebe Mark-of-the-Web e rename sem substituir um destino existente. O dlm nunca executa arquivos baixados.

## Testes

```powershell
cargo fmt --check
cargo clippy --all-targets --features custom-protocol -- -D warnings
cargo test --no-default-features --tests
node --test extension/rules.test.js
cargo run --release --no-default-features --features dev-tools --bin dlm-testserver -- 127.0.0.1:18765 67108864
cargo run --release --no-default-features --features dev-tools --bin dlm-bench -- 268435456
```

O servidor fornece bytes determinísticos e rotas de falha (`/slow.bin`, `/bad-range.bin`, `/range-200.bin`, `/drop.bin`, `/expired.bin`, `/unknown.bin`). O benchmark compara 1, 4 e 16 conexões com hash final.

Para testes isolados, defina `DLM_DATA_DIR`, `DLM_PIPE_SUFFIX` e `DLM_PORTABLE=1`. Isso separa banco/pipe e suprime registro automático do host e inicialização. `--daemon` inicia a fila sem bandeja e aceita o RPC `Shutdown`.

## Código

| Caminho | Responsabilidade |
|---|---|
| `src/engine.rs`, `fetch.rs`, `journal.rs`, `ranges.rs` | HTTP, integridade e retomada |
| `src/manager.rs` | fila, SQLite, limites e agenda |
| `src/ipc.rs`, `native.rs`, `system.rs`, `chrome.rs` | Windows, extensão e importação |
| `src/desktop.rs`, `cli.rs`, `main.rs` | janela, bandeja e comandos |
| `frontend/`, `extension/` | Svelte e extensão MV3 |
| `tests/`, `src/testserver.rs`, `src/bin/` | testes, servidor e benchmark |

## Licença

[MIT](LICENSE)
