use std::fs;
use std::path::Path;

use fat32_parser::{Fat32, FatError};

/// Test d'intégration sur une vraie image FAT32 si elle est présente
///
/// Le but est de vérifier que la lib fonctionne aussi sur un disk.img
/// formaté avec mkfs.vfat, pas uniquement sur le  volume 
/// utilisé dans les tests unitaires.
#[test]
fn read_real_disk_img_if_present() -> Result<(), FatError> {
    let img_path = Path::new("tests/disk.img");

    if !img_path.exists() {
        // On ne fait pas échouer la suite de tests si l'image n'est pas là.
        eprintln!("(info) tests/disk.img absent -> test ignoré");
        return Ok(());
    }

    let data = fs::read(img_path).map_err(|_| FatError::Other)?;
    eprintln!("(info) image chargée, taille = {} octets", data.len());

    let fs = Fat32::new(&data)?;
    let root = fs.list_root()?;
    eprintln!("(info) nombre d'entrées à la racine = {}", root.len());

    assert!(
        !root.is_empty(),
        "Racine vide, l'image de test semble mal préparée"
    );

    // Si HELLO.TXT existe, on essaye de le lire pour valider la lecture de fichier
    if let Some(entry) = root.iter().find(|e| e.name.eq_ignore_ascii_case("HELLO.TXT")) {
        eprintln!("(info) HELLO.TXT trouvé, taille déclarée = {} octets", entry.size);

        assert!(entry.is_file(), "HELLO.TXT devrait être un fichier");

        let content = fs.read_file(entry)?;
        let text = String::from_utf8_lossy(&content);
        eprintln!("(info) contenu de HELLO.TXT = {:?}", text);

        assert!(
            !text.is_empty(),
            "HELLO.TXT est présent mais vide"
        );
    } else {
        eprintln!("(info) HELLO.TXT non présent dans l'image, test limité au listage");
    }

    Ok(())
}
