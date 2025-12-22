//! Petite CLI pour explorer et modifier une image FAT32.
//!
//! Cette CLI s’appuie sur la bibliothèque `fat32_parser`:
//! - lecture: `ls`, `cat`, navigation avec `cd` et `pwd`
//! - écriture simple: `put` pour créer/écraser un fichier 8.3
//! - mode non interactif via options ou mode shell interactif
//! 
//! Exemple rapide:
//! ```
//! fat32_cli --file disk.img --ls /
//! fat32_cli --file disk.img --cat /HELLO.TXT
//! fat32_cli --file disk.img --put /NEW.TXT ./local.txt
//! ```
use fat32_parser::{Fat32, Fat32Mut};
use std::env;
use std::fs;
use std::io::{self, Write};

/// Affiche l’usage de la CLI avec les commandes disponibles.
fn print_usage() {
    eprintln!(
        "Usage:
  fat32_cli --file <disk.img> [--ls <path>] [--cat <path>] [--put <fat_path> <host_file>]

Exemples:
  fat32_cli --file disk.img --ls /
  fat32_cli --file disk.img --cat /HELLO.TXT
  fat32_cli --file disk.img --put /NEW.TXT ./local.txt

Mode shell:
  fat32_cli --file disk.img
  (puis: ls, cd, cat, put, pwd, help, exit)"
    );
}

/// Affiche l’aide du mode shell interactif.
fn print_shell_help() {
    println!(
        "Commandes:
  ls [path]            - lister un répertoire
  cat <path>           - lire un fichier
  cd [path]            - changer de répertoire courant
  put <fat_path> <src> - écrire un fichier dans l'image (persistant)
  pwd                  - afficher le répertoire courant
  help                 - cette aide
  exit                 - quitter"
    );
}

/// Point d’entrée de la CLI: parse les arguments,
/// ouvre l’image en mémoire, puis exécute la commande
/// demandée ou bascule en mode shell interactif.
fn main() {
    let mut args = env::args().skip(1);

    let mut dump_path: Option<String> = None;
    let mut command: Option<String> = None;
    let mut target_a: Option<String> = None;
    let mut target_b: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--file" | "-f" => dump_path = args.next(),
            "--ls" => {
                command = Some("ls".to_string());
                target_a = args.next();
            }
            "--cat" => {
                command = Some("cat".to_string());
                target_a = args.next();
            }
            "--put" => {
                command = Some("put".to_string());
                target_a = args.next();
                target_b = args.next();
            }
            _ => {
                eprintln!("Argument inconnu : {arg}");
                print_usage();
                return;
            }
        }
    }

    let dump_path = match dump_path {
        Some(p) => p,
        None => {
            print_usage();
            return;
        }
    };

    let mut data = match fs::read(&dump_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Impossible de lire {dump_path}: {e}");
            return;
        }
    };

    match command.as_deref() {
        Some("ls") => {
            let ro = match Fat32::new(&data) {
                Ok(fs) => fs,
                Err(e) => {
                    eprintln!("Erreur FAT32: {e:?}");
                    return;
                }
            };
            let cwd = "/";
            let path = target_a
                .as_deref()
                .map(|p| resolve_path(cwd, p))
                .unwrap_or_else(|| "/".to_string());
            run_ls(&ro, &path);
        }
        Some("cat") => {
            let ro = match Fat32::new(&data) {
                Ok(fs) => fs,
                Err(e) => {
                    eprintln!("Erreur FAT32: {e:?}");
                    return;
                }
            };
            let cwd = "/";
            let rel = match target_a {
                Some(p) => p,
                None => {
                    eprintln!("--cat nécessite un chemin");
                    print_usage();
                    return;
                }
            };
            let path = resolve_path(cwd, &rel);
            run_cat(&ro, &path);
        }
        Some("put") => {
            let fat_path = match target_a {
                Some(p) => p,
                None => {
                    eprintln!("--put nécessite un chemin FAT32");
                    print_usage();
                    return;
                }
            };
            let src = match target_b {
                Some(p) => p,
                None => {
                    eprintln!("--put nécessite un fichier source");
                    print_usage();
                    return;
                }
            };

            let content = match fs::read(&src) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("Impossible de lire {src}: {e}");
                    return;
                }
            };

            {
                let mut rw = match Fat32Mut::new(&mut data) {
                    Ok(fs) => fs,
                    Err(e) => {
                        eprintln!("Erreur FAT32: {e:?}");
                        return;
                    }
                };

                if let Err(e) = rw.write_file_by_path(&fat_path, &content) {
                    eprintln!("Erreur put {fat_path}: {e:?}");
                    return;
                }
            }

            if let Err(e) = fs::write(&dump_path, &data) {
                eprintln!("Impossible d'écrire {dump_path}: {e}");
                return;
            }

            println!("OK: {src} -> {fat_path} (image mise à jour)");
        }
        Some(other) => {
            eprintln!("Commande inconnue : {other}");
            print_usage();
        }
        None => run_shell(&dump_path, &mut data),
    }
}

/// Résout un chemin absolu ou relatif à partir d'un répertoire courant.
///
/// Exemples :
/// - current="/DIR", path=".."          -> "/"
/// - current="/DIR", path="FILE.TXT"    -> "/DIR/FILE.TXT"
/// - current="/",     path="/AUTRE/XX"  -> "/AUTRE/XX"
fn resolve_path(current: &str, path: &str) -> String {
    let mut components: Vec<String> = Vec::new();

    if path.starts_with('/') {
        for part in path.split('/') {
            push_component(&mut components, part);
        }
    } else {
        for part in current.split('/') {
            push_component(&mut components, part);
        }
        for part in path.split('/') {
            push_component(&mut components, part);
        }
    }

    if components.is_empty() {
        "/".to_string()
    } else {
        let mut result = String::from("/");
        result.push_str(&components.join("/"));
        result
    }
}

/// Ajoute un composant de chemin en gérant `.` et `..`.
fn push_component(components: &mut Vec<String>, part: &str) {
    match part {
        "" | "." => {}
        ".." => {
            components.pop();
        }
        _ => components.push(part.to_string()),
    }
}

/// Liste un répertoire et affiche une vue simple
/// (type + nom + taille) pour chaque entrée.
fn run_ls(fs: &Fat32, path: &str) {
    match fs.list_dir_path(path) {
        Ok(entries) => {
            println!("Listing de {path}:");
            for e in entries {
                let kind = if e.is_dir() { "DIR " } else { "FILE" };
                println!("{kind} {:<24} {:>8} bytes", e.name, e.size);
            }
        }
        Err(e) => eprintln!("Erreur ls {path}: {e:?}"),
    }
}

/// Lit un fichier et écrit son contenu sur la sortie standard.
fn run_cat(fs: &Fat32, path: &str) {
    match fs.read_file_by_path(path) {
        Ok(Some(bytes)) => {
            print!("{}", String::from_utf8_lossy(&bytes));
        }
        Ok(None) => eprintln!("Fichier introuvable : {path}"),
        Err(e) => eprintln!("Erreur cat {path}: {e:?}"),
    }
}

/// Lance un petit shell interactif pour manipuler l’image:
/// navigation (`cd`, `pwd`), listage (`ls`), lecture (`cat`) et écriture (`put`).
fn run_shell(img_path: &str, data: &mut Vec<u8>) {
    println!("FAT32 shell. Tapez 'help' pour l'aide, 'exit' pour quitter.");

    let stdin = io::stdin();
    let mut current_dir = String::from("/");

    loop {
        print!("fat32:{current_dir}> ");
        if io::stdout().flush().is_err() {
            break;
        }

        let mut line = String::new();
        let n = match stdin.read_line(&mut line) {
            Ok(n) => n,
            Err(_) => break,
        };
        if n == 0 {
            break;
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let mut parts = line.split_whitespace();
        let cmd = parts.next().unwrap();

        match cmd {
            "exit" | "quit" => break,
            "help" => print_shell_help(),
            "pwd" => println!("{current_dir}"),
            "ls" => {
                let ro = match Fat32::new(&data) {
                    Ok(fs) => fs,
                    Err(e) => {
                        println!("Erreur FAT32: {e:?}");
                        continue;
                    }
                };

                let path = if let Some(p) = parts.next() {
                    resolve_path(&current_dir, p)
                } else {
                    current_dir.clone()
                };
                run_ls(&ro, &path);
            }
            "cat" => {
                let ro = match Fat32::new(&data) {
                    Ok(fs) => fs,
                    Err(e) => {
                        println!("Erreur FAT32: {e:?}");
                        continue;
                    }
                };

                if let Some(p) = parts.next() {
                    let path = resolve_path(&current_dir, p);
                    run_cat(&ro, &path);
                } else {
                    println!("Usage: cat <path>");
                }
            }
            "cd" => {
                let ro = match Fat32::new(&data) {
                    Ok(fs) => fs,
                    Err(e) => {
                        println!("Erreur FAT32: {e:?}");
                        continue;
                    }
                };

                let target = if let Some(p) = parts.next() {
                    resolve_path(&current_dir, p)
                } else {
                    "/".to_string()
                };

                match ro.open_path(&target) {
                    Ok(Some(entry)) if entry.is_dir() => current_dir = target,
                    Ok(Some(_)) => println!("{target} n'est pas un répertoire"),
                    Ok(None) => println!("Répertoire introuvable : {target}"),
                    Err(e) => println!("Erreur cd vers {target}: {e:?}"),
                }
            }
            "put" => {
                let fat_path = match parts.next() {
                    Some(p) => resolve_path(&current_dir, p),
                    None => {
                        println!("Usage: put <fat_path> <src_file>");
                        continue;
                    }
                };
                let src = match parts.next() {
                    Some(p) => p.to_string(),
                    None => {
                        println!("Usage: put <fat_path> <src_file>");
                        continue;
                    }
                };

                let content = match fs::read(&src) {
                    Ok(v) => v,
                    Err(e) => {
                        println!("Impossible de lire {src}: {e}");
                        continue;
                    }
                };

                {
                    let mut rw = match Fat32Mut::new(data) {
                        Ok(fs) => fs,
                        Err(e) => {
                            println!("Erreur FAT32: {e:?}");
                            continue;
                        }
                    };

                    if let Err(e) = rw.write_file_by_path(&fat_path, &content) {
                        println!("Erreur put {fat_path}: {e:?}");
                        continue;
                    }
                }

                if let Err(e) = fs::write(img_path, &*data) {
                    println!("Impossible d'écrire {img_path}: {e}");
                    continue;
                }

                println!("OK: {src} -> {fat_path} (image mise à jour)");
            }
            _ => println!("Commande inconnue: {cmd}. Tapez 'help'."),
        }
    }
}

#[cfg(test)]
mod cli_path_tests {
    use super::resolve_path;

    #[test]
    fn chemin_parent_depuis_dir() {
        let r = resolve_path("/DIR", "..");
        assert_eq!(r, "/");
    }

    #[test]
    fn chemin_courant_point_file() {
        let r = resolve_path("/DIR", "./FILE.TXT");
        assert_eq!(r, "/DIR/FILE.TXT");
    }

    #[test]
    fn chemin_absolu_ignore_courant() {
        let r = resolve_path("/DIR", "/AUTRE/XX");
        assert_eq!(r, "/AUTRE/XX");
    }
}
