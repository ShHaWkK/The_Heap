# The_Heap


## Ce que j’ai fait
- J’ai écrit un noyau `no_std` qui boote et affiche une bannière sur VGA et sur le port série (COM1).
- J’ai installé un allocateur global basé sur “slabs” et validé l’allocation avec `Vec`/`String`.
- J’ai intégré un parseur FAT32 en RAM : listage racine, lecture d’un fichier, écriture (création/overwrite) et relistage après écriture.
- J’ai ajouté un `panic_handler` qui affiche le message et la localisation sur VGA et sur la série, sans allocation.
- J’ai nettoyé les warnings et rendu l’expérience de lancement simple via `cargo run` au niveau du crate kernel (runner bootimage + cible JSON).

## Démo FAT32 côté noyau
- Image RAM minimale avec `HELLO.TXT` et un répertoire `DIR`.
- Au boot : listage ROOT, lecture de `HELLO.TXT`, écriture de `NEW.TXT`, nouveau listage ROOT, lecture de `NEW.TXT`.
- Côté code, j’ai réutilisé `Fat32Mut::as_read()` pour éviter de reparser plusieurs fois et garder le coût au strict minimum.

## Fonctions intéressantes (façon de penser)
- VGA – scroll et `clear` : j’avance ligne par ligne; à la 25e, je recopie la zone écran vers le haut puis je nettoie la dernière ligne. C’est bête et linéaire, mais prévisible et sûr.
- Série – `serial_println_args` : j’écris directement des `format_args!` sur COM1, sans allocation, via une implémentation minimaliste de `Write`. Ça rend les logs robustes dès le boot.
- FAT32 – `open_path` : je découpe le chemin par `/`, normalise en majuscules (noms courts), liste le répertoire courant, compare, puis j’avance cluster par cluster. Le but est la lisibilité avant tout.
- FAT32 – `write_file_by_path` : je sépare parent/fichier, valide le nom court 8.3, trouve une entrée existante, libère les anciens clusters en FAT si besoin, alloue la chaîne requise, écris les bytes, puis mets à jour l’entrée (taille + cluster). La version V1 ne crée pas de nouveaux clusters de répertoire : c’est volontairement simple.
- Allocateur – “slabs + bump” : pour les petites tailles, je découpe des pages 4K en blocs homogènes avec freelists (LIFO); pour les grosses tailles, je prends la voie “bump” sans recyclage en V1. C’est un bon compromis pour un noyau d’examen.
- Panic handler – sans allocation : il reconfigure COM1, colore VGA en rouge, et imprime message + localisation. L’idée est d’avoir un signal clair au pire moment, sans dépendre du heap.


## Qualité / Bonus
- Clippy strict et corrections (usage de `div_ceil`, simplifications).
- Miri pour durcir la vérification mémoire (sur les tests, pas sur le kernel) :
- Runner `cargo run` côté kernel (via alias `.cargo/config.toml`).

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

```bash
sudo apt install cargo -y
sudo apt install rustup -y
rustup toolchain install nightly
rustup component add rust-src llvm-tools-preview --toolchain nightly
cargo install bootimage
sudo apt update && sudo apt install -y qemu-system-x86
```

- Permissions éviter sudo:
  Pourquoi: `sudo` change le PATH et l’environnement; l’outil `bootimage` installé pour l’utilisateur peut ne pas être trouvé en root. De plus, `sudo` crée des fichiers appartenant à root dans le projet et casse les builds/tests ultérieurs.
  Solution : réattribuer le projet à l’utilisateur avant d’utiliser Cargo, puis ne pas utiliser `sudo`:
  
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

