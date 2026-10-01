# Audit outillé (2026-10-01)

Base : `main` @ `c54c502`. Branche : `test/audit-outille`. Toolchain : stable 1.96.0, nightly (miri, udeps, cargo-fuzz).
Aucun code existant modifié ; aucun accès au matériel. Tests nouveaux : `apihub-app/audit-props/` (proptest, crate
hors workspace, `cd apihub-app/audit-props && cargo test`) et `apihub-app/audit-fuzz/` (cargo-fuzz, nightly).

## Synthèse

| Outil | Résultat chiffré | Réel / bruit |
|---|---|---|
| clippy pédantique (code de production, lib+bin) | 1 149 diagnostics uniques ; 1 937 avec `--all-targets` (tests inclus) | 0 panique réelle trouvée dans le code de production (voir §1) ; le reste est stylistique |
| cargo audit | 5 vulnérabilités + 6 avertissements (450 crates) | réel mais exposition faible, tout via `eframe` → #224 |
| cargo deny | advisories FAILED, licenses FAILED, bans OK (27 doublons), sources OK | champ `license` absent (5 crates) → #225 |
| cargo geiger | akm-core : 30 expressions unsafe ; aucun crate n'a `#![forbid(unsafe_code)]` | 18 blocs unsafe de production, tous revus, aucun défaut de mémoire (§3) |
| cargo machete / udeps | machete : `serde` inutile dans apihub-app ; udeps : « all deps used » | machete a raison (aucun `serde::` ni `derive` dans `src/`), udeps faux négatif → #225 |
| cargo llvm-cov | **73,74 % lignes** (13 647 lignes, 3 586 manquées ; régions 75,02 %, fonctions 73,18 %), 306 tests passent, 3 ignorés | akm-core 92-100 % sauf `hidraw.rs` 44,8 % et `read_policy.rs` 82,6 % |
| cargo mutants (9 fichiers akm-core) | 799 mutants : **631 tués, 120 survivants, 48 non viables** → score 84,0 % | survivants = tests faibles (§6) |
| proptest (4 fichiers, 28 tests actifs + 8 ignorés) | 5 défauts reproduits (8 tests `#[ignore]`) ; 28 tests verts | #220 #221 #222 #223 |
| cargo-fuzz (4 cibles, 45 s chacune) | 1,09 M (decode) + 1,79 M (config) + 2,26 M (history) + 18,6 M (rdesc) exécutions, 0 crash | pas de panique dans les décodeurs d'octets |
| miri | 8 tests passent (ioctl sur fd invalide, états de réveil, décodage passif) ; `poll`, `mkfifo` non supportés par Miri | limite de l'outil, pas un défaut |

Défauts confirmés (tous avec test reproducteur `#[ignore]` pointant l'issue) :

| # | Gravité | Défaut |
|---|---|---|
| #220 | faible | `Fixture::from_hex_dump("aéa")` panique (tranchage hors frontière de char) |
| #221 | faible | `alias::validate` accepte U+061C (marque bidi), U+3164, U+2800, U+00AD, tags U+E00xx |
| #222 | moyenne | un octet non UTF-8 dans `history.jsonl` : `read()` renvoie 0 entrée, `rotate()` en erreur à vie |
| #223 | faible | `history::parse` sans validation : débordement u64 dans `forecast::estimate`, taux `inf` dans `estimate_remaining` |
| #224 | faible (exposition) | 5 avis RustSec via eframe 0.29 |
| #225 | hygiène | `license` absent, `serde` inutile |

## 1. clippy (pedantic + nursery + unwrap/expect/indexing/panic/arithmetic)

Commande exacte de la mission, puis relance sans `--all-targets` pour isoler le code de production.

Production (1 149 diagnostics uniques) : `doc_markdown` 188, `must_use_candidate` 178, `missing_const_for_fn` 92, `use_self` 87,
`arithmetic_side_effects` 80, `indexing_slicing` 54, `redundant_closure_for_method_calls` 45, `unused_self` 42,
`missing_errors_doc` 38, `uninlined_format_args` 32, `expect_used` 12, `cast_*` 40, `unwrap_used` 2, `panic` 0.
Avec les tests : 464 `unwrap`, 166 `indexing`, 36 `expect` uniques (quasi tout dans `#[cfg(test)]`).

Tri :
- **`indexing_slicing` (54) examinés un par un** : `decode.rs` 303-390 (`b[1..]`, `buf[1..]`), `read_policy.rs` 220-231, `calibration.rs` 42/76-82,
  `chemistry.rs` 150-152, `hidraw.rs` 39/322, `passive.rs` 264, `discover.rs` 46, `history.rs` 206-207, `rssi.rs` 238 : tous précédés d'une
  garde (`len() >= n`, trames vides filtrées par `Reader::get`, `windows(2)`, `n <= buf.len()` par construction). Confirmé par 4 000 cas
  proptest + 1,09 M exécutions libFuzzer + troncature de chaque trame réelle A1314 à toutes les longueurs : **aucune panique**.
- `src/main.rs:494-495` (`first().unwrap()`) : gardé par un `return` si trop peu de points. Idem `unwrap` de `OwnedObjectPath::try_from("/MenuBar")` (constante valide).
- `expect` de production (12) : constantes (`BUS_NAME.try_into().expect("valid name")`), `spawn` de thread, `"just pushed"` : acceptables.
- `arithmetic_side_effects` (80) : le seul débordement réel est `forecast.rs` `last_ts + ...` (#223). Les autres opèrent sur des durées/indices bornés.
- Bruit ignoré : `doc_markdown`, `must_use_candidate`, `missing_const_for_fn`, `use_self`, casts d'affichage egui.

## 2. cargo audit / cargo deny

cargo audit (base RustSec du 2026-10-01, 1 278 avis) : RUSTSEC-2026-0194 et 0195 (quick-xml 0.30.0 et 0.39.2, 7.5), RUSTSEC-2026-0257
(webbrowser 1.2.0, injection d'argument via `BROWSER`). Avertissements : paste, ttf-parser (inmaintenus), anyhow 1.0.102, event-listener 5.4.1,
memmap2 0.9.10, rand 0.8.5 (unsound). Chemins : tous sous `apihub-app → eframe 0.29.1` ; quick-xml 0.39.2 n'est qu'un proc-macro de compilation
(`wayland-scanner`), webbrowser n'est appelé que par `open_url` (aucun lien dans l'UI). → #224.

cargo deny (config permissive MIT/Apache/BSD/ISC/Unicode/Zlib/MPL/OFL) : licences connues toutes acceptées ; `error[unlicensed]` pour `akm-core`,
`apihub-app`, `apple-kb-monitord` (+ `no-license-field` pour `akmctl`, `akm-helper`), dépôt en GPL-2.0 ; 27 doublons de versions (zbus 4/5,
thiserror 1/2...) imputables à eframe/zbus ; sources : uniquement crates.io. → #225.

## 3. unsafe : inventaire et revue manuelle

cargo geiger (akm-core) : 30/30 expressions unsafe, 0 fonction/impl/trait unsafe. Décompte à la main hors tests : `hidraw.rs` 8, `passive.rs` 3,
`read_policy.rs` 1, `apple-kb-monitord/main.rs` 1 (2 appels `signal`), `akm-helper` 3 (`umask`, `fchown`, `geteuid`), `akmctl/doctor.rs` 1 (`geteuid`),
`apihub-app/src/main.rs` 1 (`time`). Aucune crate ne déclare `#![forbid(unsafe_code)]` (et `power.rs` annonce « No unsafe » : exact).

| Bloc | Verdict |
|---|---|
| `hidraw.rs:35` `ioctl(fd, HIDIOCGFEATURE, buf)` | Sain. `0xC1004807` = `_IOWR('H', 7, 256)` (dir 3, taille 0x100, type 0x48, nr 7) ; tampon de 256 octets, `ret >= 0` avant `as usize`, `min(buf.len())`, EINTR borné à 3, errno lu juste après l'appel. |
| `hidraw.rs:143` `fcntl(F_GETFD)` sur le fd en cache | Sans effet mémoire, mais **ne détecte pas la réutilisation d'un numéro de fd** fermé ailleurs. Le fd est copié hors du `Mutex` (`get_hid_fd` renvoie `(c_int, String)`) : une fermeture concurrente (`close_hid_fd`) pendant `read_keyboard` donnerait un `ioctl` sur un fd fermé ou réutilisé. Dans le démon les appels sont séquentiels (acteur puis superviseur) : risque théorique, non reproductible. Suggestion : `OwnedFd` + `Arc`. |
| `hidraw.rs:151` `open(O_RDWR | O_CLOEXEC)` | CString valide, fd testé `< 0`. **Durcissement** : `O_RDWR` donne la capacité d'écriture (SET_REPORT) alors que le code ne fait que lire ; le moniteur de réveil et l'écoute passive utilisent déjà `O_RDONLY`. Non éprouvé sur matériel (interdit ici). |
| `hidraw.rs:164,327` `close` | Fd issu du cache ou de l'`open` local ; fermé sur toutes les sorties de `wake_loop`. Sain. |
| `hidraw.rs:277,294,309` `open`/`poll`/`read` (wake) | `pollfd` unique, `read` borné à 64 octets, `n < 0/0` traités, `POLLHUP/ERR/NVAL` sortent (pas de boucle à 100 %). Sain. |
| `passive.rs:190,241,253`, `read_policy.rs:129` `flock`/`poll`/`read` | Fd possédés par `File`/paramètre, tampons locaux, EINTR/EAGAIN gérés. Sain. |
| `apple-kb-monitord/main.rs:177` `signal(SIGTERM/SIGINT)` | Le gestionnaire ne fait qu'un `store` atomique (async-signal-safe). Le retour (`SIG_ERR`) n'est pas vérifié ; `sigaction` serait plus portable. Mineur. |
| `akm-helper:108,316-324` `umask` (déclaré en `extern "C"` à la main) | Correct (`mode_t` = u32 sous Linux) mais redondant avec `libc::umask`. Les tests qui changent l'umask du processus sont sérialisés par `UMASK` mais pas contre les autres tests du binaire. |
| `akm-helper:158-160` `fchown(fd,0,0)` / `geteuid` | Fd possédé ; l'échec n'est une erreur que si `euid == 0` : cohérent. Mode explicite 0644, `O_NOFOLLOW|O_EXCL`, rename atomique, fsync du dossier : bon. |
| `src/main.rs:471` `libc::time` | Sain ; remplaçable par `SystemTime` (safe). |

Helper (surface privilégiée) : grammaire d'arguments stricte (`set-fnmode` + 1 chiffre ASCII 0-3 + `--persist`), aucun chemin ni variable
d'environnement pris en entrée, symlinks refusés. Pas de défaut trouvé. `merge_fnmode`/`parse_args` sont privées au binaire donc
non testables depuis l'extérieur : extraire un `lib.rs` permettrait de les fuzzer.
Note : `read_policy::lock_path()` retombe sur `/tmp/apple-kb-monitor/hid.lock` si `XDG_RUNTIME_DIR` est absent ; chemin prévisible dans un
dossier partagé (déni de lecture possible par un autre utilisateur, ouverture sans `O_NOFOLLOW`). Sous `systemd --user` la variable est
toujours définie : non retenu comme défaut.

Miri (`cargo +nightly miri test -p akm-core --lib -- hidraw:: passive::tests read_policy::`, `-Zmiri-disable-isolation`) : 8 tests exécutés sans UB
(dont `hid_read_feature_on_invalid_fd_reports_its_own_errno`) ; l'exécution s'arrête sur `poll` puis `mkfifo`, non supportés par Miri.

## 4. Couverture (llvm-cov, workspace sans apihub-app)

Total : 73,74 % de lignes (13 647, 3 586 manquées), 75,02 % des régions (24 125), 73,18 % des fonctions (1 741). Zones critiques :

| Zone | Lignes | Commentaire |
|---|---|---|
| decode 98,1 · alerts 96,1 · machine 97,8 · chemistry 98,9 · forecast 98,2 · history 96,8 · config 97,3 · discover 97,4 · alias 100 · batteries 99,2 · recovery 95,6 | | Bien couvert (mais voir §6 : couvert ≠ vérifié) |
| `akm-core/src/hidraw.rs` | 44,8 % | partie matérielle (ioctl réel, fd persistant, boucle de réveil) : seul l'EBADF est testé |
| `akm-core/src/read_policy.rs` | 82,6 % | verrou inter-processus (`try_lock`) et espacement jamais exercés avant l'audit |
| `akm-helper/src/main.rs` | 89,3 % | OK ; `main()` non couvert |
| `apple-kb-monitord` : `watcher.rs` 0 %, `tray/sni.rs` 0 %, `tray.rs` 8,3 %, `sleep.rs` 36,5 %, `client.rs` 53 %, `repair.rs` 56 %, `main.rs` 49 % | | écoute BlueZ/UPower, SNI, veille système : non testés sans bus réel |
| `akmctl` : `main.rs` 6,7 %, `bus.rs` 16 %, `repair.rs` 22 %, `doctor.rs` 56,6 % | | `repair` (actions correctives) est le moins couvert |

## 5. Propriétés et fuzzing

`apihub-app/audit-props/tests/` (28 tests verts dont 8 de `policy_mutants.rs` ; 8 tests `#[ignore]` figent 5 défauts, 4 issues ; `cargo test -- --include-ignored` les fait échouer) :

- `decode_props.rs` : décodeur BCM2042 sur 4 000 tables de trames aléatoires/tronquées (jamais de panique, % ∈ [0,100], mV ∈ `PLAUSIBLE_MV`, estimation finie, nom sans NUL, `0x4C` jamais publié sans option) ; **lecture sûre : seuls 0x47/0x46/0x49 sont demandés** ; troncature de chaque trame réelle A1314.
- `parsers_props.rs` : `config.toml` (seuils triés/uniques/1..=99, hystérésis finie dans 1..=20), historique JSONL (texte quelconque, nombres extrêmes), descripteur HID (tout préfixe), rapports passifs/réveil/calibration, alias, calibration (jamais NaN/∞, bornée, monotone).
- `logic_props.rs` : machine d'états de connexion sur 3 000 séquences d'événements (jamais d'état impossible : déconnectée ⇒ ni mac, ni file, ni échéance, ni action ; jamais bloquée ; pas de doublon suivi/attente), récupération de liaison (jamais deux `Connect` à moins de 20 s), alertes, chimie (bornée, monotone, `low ≤ pct ≤ high`), historique/prévision.
- `policy_mutants.rs` : 8 tests ajoutés pour les survivants de §6 (espacement `MIN_GAP`, liste blanche, budget de 2 s, porte d'activité 60 s, verrou `flock` exclusif puis libéré, valeurs exactes de l'autonomie/prévision/dates).

cargo-fuzz (`cd apihub-app && cargo +nightly fuzz run --fuzz-dir audit-fuzz <decode|config|history|rdesc>`) : 0 crash en 45 s par cible. Le fuzz
d'octets n'atteint pas les défauts #222/#223 (ils exigent un JSON valide aux valeurs extrêmes) : trouvés par proptest.

## 6. Mutation (cargo mutants, 9 fichiers d'akm-core, 799 mutants, 9 min, -j6)

| Fichier | Tués | Survivants |
|---|---|---|
| decode.rs | 62 | 4 (`Fixture::is_empty`, garde `!b.is_empty()`, garde `rd.stopped`) |
| calibration.rs | 75 | 5 (`>`→`>=` dans `calibration_valid`, `+`→`-` ligne 79) |
| read_policy.rs | 32 | **23** |
| machine.rs | 58 | 5 (lignes 14, 159, 204, 239) |
| recovery.rs | 91 | **38** |
| alerts.rs | 25 | 2 (`<`→`<=` ligne 144) |
| chemistry.rs | 45 | 3 |
| forecast.rs | 68 | 11 (arithmétique de la régression lignes 96-127 : taux/ajustement non vérifiés numériquement) |
| history.rs | 175 | 29 (`estimate_remaining` remplacé par n'importe quel tuple, `format_utc` lignes 133, garde `NotFound`, `read_since`) |

Survivants à risque : (1) **`read_policy`** : `note_input → ()`, `try_lock → None`, `lock_path → Default`, l'espacement `MIN_GAP`, le budget et les bornes de
`last_input_age` ne sont vérifiés par aucun test, alors que cette politique protège le clavier du gel (#177) ; (2) `forecast` et
`estimate_remaining` : aucun test ne vérifie un taux ou une date calculés ; (3) `recovery` : accesseurs (`attempts`, `failures`, `last_error`, `since`) et
classification de `ConnectError` (`InProgress/Busy`, `NotReady`) ; (4) `Machine` ligne 239 (`delete !` dans `reconcile`). `policy_mutants.rs` cible
(1) et (2) ; non re-mesuré par cargo-mutants (crate de tests externe au workspace).

## Reproduction

```
export CARGO_TARGET_DIR=<dossier propre>
cd apihub-app
cargo clippy --workspace --all-targets -- -W clippy::pedantic -W clippy::nursery -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic -W clippy::arithmetic_side_effects
cargo audit; cargo deny check; cargo machete; cargo +nightly udeps --workspace --all-targets
cargo llvm-cov --workspace --exclude apihub-app --summary-only
cargo mutants -p akm-core -f akm-core/src/{decode,calibration,read_policy,machine,recovery,alerts,chemistry,forecast,history}.rs
(cd audit-props && cargo test)
```
Note cargo-mutants : le dépôt copie seulement `apihub-app/` ; les `include_str!("../../../tests/...")` exigent des liens symboliques `tests/`, `keyd/` dans `$TMPDIR`.
