# The_Heap – mon projet

## Pourquoi / mon but
- Je voulais comprendre et implémenter un allocateur mémoire simple en environnement `no_std`, puis m’en servir dans un mini noyau Rust qui boote sous QEMU.
- Mon objectif était d'écrire un allocateur par slabs lisible et documenté, et prouver son intégration réelle dans un noyau (sorties VGA/série, panic propre, petite démo FAT32 en RAM).

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

## Commandes utiles

### Ce que vous verrez
- Fenêtre VGA :
  - bannière “The Heap - kernel”
  - listage ROOT
  - contenu de `HELLO.TXT`
  - second listage après écriture `NEW.TXT`
- Terminal (si `-serial stdio`) :
  - “The Heap - kernel”
  - “ROOT: HELLO.TXT DIR”
  - “HELLO”
  - “ROOT (apres ecriture): HELLO.TXT DIR NEW.TXT”
  - “NEW!”
  - “The Heap: allocator OK”


## Qualité / Bonus
- Clippy strict et corrections (usage de `div_ceil`, simplifications).
- Miri pour durcir la vérification mémoire (sur les tests, pas sur le kernel) :
- Runner `cargo run` côté kernel (via alias `.cargo/config.toml`).

## Commandes

```bash
sudo chown -R $(whoami):$(whoami) /chemin/vers/ton/projet/
```
git bundle create the_heap.bundle --all

### Pré‑requis

```bash
sudo apt install cargo -y
sudo apt install rustup -y
rustup toolchain install nightly
rustup component add rust-src llvm-tools-preview --toolchain nightly
cargo install bootimage
sudo apt update && sudo apt install -y qemu-system-x86
```
### Build 

```bash
cargo build
```


### Tests du workspace

```bash
cargo test
```

Par crate :

```bash
cargo test -p slaballoc
cargo test -p fat32_parser
```

Doc tests :

```bash
cargo test -p slaballoc --doc
cargo test -p fat32_parser --doc
```

Voir les sorties :

```bash
cargo test -- --nocapture
```

### Tests kernel (QEMU)

```bash
cargo test -p kernel --target x86_64-the_heap.json
# ou simplement (alias):
cargo ktest
```

Note : la configuration `build-std` est définie via les alias dans `.cargo/config.toml` (inclut `panic_abort`). Si vous voyez une erreur “duplicate lang item in crate core: sized”, 
exécutez :

```bash
cargo clean
cargo ktest
```

#### Dépannage kernel
- Ne pas utiliser `sudo` pour `cargo krun/ktest` : le runner `bootimage` peut ne pas être trouvé dans le `PATH` root.
- Vérifier que `qemu-system-x86_64` est installé (paquet `qemu-system-x86`).
- Mettre à jour le nightly et les composants : `rustup update nightly && rustup component add rust-src llvm-tools-preview --toolchain nightly`.

### Lancer la démo kernel

```bash
cargo krun
# ou:
cd kernel && cargo run
```

## Crates – Vue d’ensemble

### Crate slaballoc
-  allocateur global `no_std` par slabs, protégé par spinlock.
- Concepts clés :
  - Classes de tailles discrètes et freelists LIFO pour les petites allocs.
  - Bump allocator pour les tailles > 4096 octets (V1, pas de recyclage).
  - Respect de `GlobalAlloc` et des contraintes d’alignement.
- Exemple (Hello world de l’allocateur) :

```rust
use slaballoc::LockedAlloc;
use core::alloc::Layout;

// Dans un contexte test/doc : on réserve un buffer pour le heap.
let mut heap = vec![0u8; 64 * 1024];
let alloc = LockedAlloc::new();
unsafe { alloc.init(heap.as_mut_ptr() as usize, heap.len()) };

// On alloue puis désalloue un bloc.
let layout = Layout::from_size_align(32, 8).unwrap();
let p = unsafe { alloc.alloc(layout) };
assert!(!p.is_null());
unsafe { alloc.dealloc(p, layout) };
```

### Crate fat32_parser
- Résumé : parseur FAT32 en mémoire (lecture + écriture simple), noms courts 8.3.
- Concepts clés :
  - Vue lecture seule (`Fat32`) et lecture/écriture (`Fat32Mut`) sur `&[u8]` / `&mut [u8]`.
  - Résolution de chemin par segments, listage d’entrées 32 octets, chaîne de clusters via FAT.
  - Écriture V1 : création/overwrite de fichier, pas de création de répertoires.
- Exemple (Hello world du parseur) :

```rust
// Exemple compilable (peut ne pas s’exécuter sur un buffer vide) :
let mut disk = vec![0u8; 10 * 512];
let mut rw = fat32_parser::Fat32Mut::new(&mut disk)?;
rw.write_file_by_path("/NEW.TXT", b"DATA")?;
let ro = rw.as_read();
let got = ro.read_file_by_path("/NEW.TXT")?.unwrap();
assert_eq!(got, b"DATA");
# Ok::<(), fat32_parser::FatError>(())
```

## Choix 
- Le périmètre est volontairement réduit pour un examen : pas de LFN (noms longs), pas de `mkdir`, pas de timestamps. Je n’écris que des fichiers 8.3 et je liste/ouvre des répertoires existants. Moins de surface = plus de robustesse.
- Overwrite libère proprement l’ancienne chaîne de clusters avant de réécrire. Ça évite les fuites dans la FAT et garde le volume lisible.
- J’ai mis une limite de sécurité sur le nombre de clusters traversés en lecture pour éviter les boucles infinies sur une image cassée. Lisibilité et sûreté > performance.
- L’allocateur est “slabs + bump” : freelists LIFO pour les petites tailles, bump sans recyclage pour les grosses. En V1, c’est simple et fiable. La fragmentation des grosses tailles est acceptable dans le contexte.
- J’ai préféré des chemins “droits” et explicites : la résolution de chemin avance segment par segment, compare en majuscules (8.3), lit le répertoire, passe au suivant. Le but est que n’importe qui puisse suivre sans surprises.


- Début par l’allocateur : j’ai validé les classes de tailles et l’alignement avec des tests simples, puis j’ai monté le spinlock pour le rendre utilisable globalement.
- Boot du noyau : sortie VGA texte et COM1 pour que le debug soit possible sans allocation.
- FAT32 lecture : parsing du BPB, calcul des offsets, lecture FAT, reconstitution des entrées 8.3, listage du root, lecture d’un fichier.
- FAT32 écriture simple : allouer des clusters libres, chaîner en FAT, écrire la data, mettre à jour l’entrée. Pas de création de répertoires (périmètre maîtrisé).
- Rustdoc/doc tests : j’ai documenté les API avec des exemples compilables et corrigé une erreur de doctest (il fallait importer `GlobalAlloc` dans l’exemple de l’allocateur).

## Doutes, ratés et corrections
- Doctest `LockedAlloc`: l’exemple ne compilait pas, car les méthodes de `GlobalAlloc` ne sont visibles que si le trait est en scope. 
Correction : `use core::alloc::GlobalAlloc` dans l’exemple.
- Lecture de chaînes FAT : risque de boucle infinie si l’image est incohérente. 
Correction : borne maximale sur le nombre de clusters et filtrage des valeurs EOC.
- Alignements : certaines allocations échouaient si l’alignement dépassait la taille. 
Correction : `align_up` systématique et prise en compte de `max(size, align)` pour choisir la classe.

