# The_Heap

## Architecture du dépôt
- Crates:
  - kernel: noyau `no_std`, boot via bootimage/QEMU, sorties VGA et COM1, démo FAT32 et allocateur global.
  - slaballoc: allocateur par slabs (freelists LIFO) + chemin “bump” pour grandes tailles; respecte `GlobalAlloc`.
  - fat32_parser: lib `no_std` (hors tests) pour lire/écrire une image FAT32 en RAM (noms courts 8.3).
- Cible et runner:
  - x86_64-the_heap.json: cible bare‑metal (panic=abort).
  - `.cargo/config.toml`: alias kbuild/ktest/krun et runner “bootimage runner”.

## Démo au démarrage (ce que vous verrez)
- VGA:
  - “The Heap - kernel”
  - Listage de la racine
  - Contenu de `HELLO.TXT`
  - Nouveau listage après écriture `NEW.TXT`
- Série (-serial stdio):
  - “The Heap - kernel”
  - “ROOT: HELLO.TXT DIR”
  - “HELLO”
  - “ROOT (apres ecriture): HELLO.TXT DIR NEW.TXT”
  - “NEW!”
  - “The Heap: allocator OK”

## Construire et lancer
- Prérequis:
  - Rust nightly + composants: rust-src, llvm-tools-preview
  - bootimage
  - qemu-system-x86_64
- Permissions éviter sudo:
Pourquoi: `sudo` change le PATH et l’environnement; l’outil `bootimage` installé pour l’utilisateur peut ne pas être trouvé en root. De plus, `sudo` crée des fichiers appartenant à root dans le projet et casse les builds/tests ultérieurs.
  Remède: réattribuer le projet à l’utilisateur avant d’utiliser Cargo, puis ne pas utiliser `sudo`:
  ```bash
  sudo chown -R $(whoami):$(whoami) /chemin/vers/ton/projet/
  ```
  - Ensuite, exécuter `cargo ktest`/`cargo krun` sans `sudo`.
- Build:
  - `cargo build`
- Tests workspace:
  - `cargo test`
  - Par crate: `cargo test -p slaballoc`, `cargo test -p fat32_parser`
  - Doc tests: `cargo test -p slaballoc --doc`, `cargo test -p fat32_parser --doc`
- Kernel (QEMU):
  - Tests: `cargo ktest`
  - Démo: `cargo krun`
- Dépannage:
  - Si “duplicate lang item core: sized” → `cargo clean` puis `cargo ktest`
  - Ne pas utiliser `sudo` pour krun/ktest
  - Installer QEMU si “program not found”


## Git bundle (pour moi oublie à chaque fois la commande)
  - `git bundle create the_heap.bundle --all`

## Ressources
- Slab Allocator (Linux): https://www.kernel.org/doc/gorman/html/understand/understand011.html
- Learning Rust With Entirely Too Many Linked Lists: https://rust-unofficial.github.io/too-many-lists/
- Tutoriel OS 64‑bit (QEMU/LLD): https://github.com/gmarino2048/64bit-os-tutorial
- Bonwick – The Slab Allocator, an Object Caching Kernel Memory Allocator

