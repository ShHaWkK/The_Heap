//! Parseur FAT32 (lecture + écriture simple).
//!
//! Ce crate manipule un volume FAT32 directement depuis un buffer mémoire.
//! Il permet :
//! - de lister des répertoires et lire des fichiers (lecture),
//! - de créer ou écraser un fichier 8.3 et écrire ses données (écriture simple),
//!   en modifiant réellement le buffer du “disque”.
//!
//! Notes importantes :
//! - Le cœur est en `no_std` (hors tests) et n’utilise que `core` et `alloc`.
//! - L’écriture vise uniquement les noms courts FAT (format 8.3). Pas de LFN.
//! - On ne gère pas la création de répertoires (le parent doit déjà exister).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::{string::String, vec::Vec};

mod dir_entry;

pub use dir_entry::{Attributes, DirEntry};

/// Erreurs possibles lors de l’accès à un volume FAT32.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatError {
    /// Le buffer ne contient pas assez de données pour un volume valide.
    BufferTooSmall,
    /// Les champs de l'en-tête ne correspondent pas à un volume FAT32 attendu.
    NotFat32,
    /// Tentative de lecture/écriture en dehors du buffer.
    OutOfBounds,
    /// Numéro de cluster invalide (ex: < 2).
    InvalidCluster,
    /// On tente de lire un répertoire comme un fichier.
    NotAFile,
    /// On tente de lister un fichier comme un répertoire.
    NotADirectory,
    /// Le chemin ne correspond à aucune entrée connue.
    PathNotFound,
    /// Nom non supporté (pas un 8.3 simple).
    InvalidName,
    /// Plus de place (pas assez de clusters libres ou pas de slot de dir libre).
    NoSpaceLeft,
    /// Erreur générique (ex: chemin relatif).
    Other,
}

/// Valeur “End Of Chain” en FAT32.
/// En pratique on considère EOC si `>= 0x0FFF_FFF8`.
const FAT32_EOC: u32 = 0x0FFF_FFFF;

/// Vue en lecture seule d’un volume FAT32 stocké dans un buffer mémoire.
///
/// Cette vue n’écrit jamais dans l’image.
#[derive(Debug)]
pub struct Fat32<'a> {
    disk: &'a [u8],
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    sectors_per_fat: u32,
    root_cluster: u32,
}

/// Vue en lecture/écriture d’un volume FAT32 stocké dans un buffer mémoire.
///
/// Les opérations modifient directement `disk`.
/// Si tu sauvegardes ce buffer dans un fichier (`disk.img`), la modification est persistante.
#[derive(Debug)]
pub struct Fat32Mut<'a> {
    disk: &'a mut [u8],
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    sectors_per_fat: u32,
    root_cluster: u32,
}

impl<'a> Fat32<'a> {
    /// Construit une vue FAT32 depuis un dump en mémoire (lecture seule).
    ///
    /// On lit un BPB minimal et on récupère les paramètres indispensables
    /// pour calculer les offsets (FAT, data, clusters).
    pub fn new(disk: &'a [u8]) -> Result<Self, FatError> {
        let p = parse_bpb(disk)?;
        Ok(Self {
            disk,
            bytes_per_sector: p.bytes_per_sector,
            sectors_per_cluster: p.sectors_per_cluster,
            reserved_sectors: p.reserved_sectors,
            num_fats: p.num_fats,
            sectors_per_fat: p.sectors_per_fat,
            root_cluster: p.root_cluster,
        })
    }

    /// Liste le contenu du répertoire racine.
    pub fn list_root(&self) -> Result<Vec<DirEntry>, FatError> {
        self.list_dir_cluster(self.root_cluster)
    }

    /// Liste un répertoire à partir d’un chemin absolu (ex: `"/DIR"`).
    ///
    /// - `"/"` liste la racine
    /// - si le chemin cible un fichier, on retourne `NotADirectory`
    pub fn list_dir_path(&self, path: &str) -> Result<Vec<DirEntry>, FatError> {
        if path == "/" {
            return self.list_root();
        }

        let entry = self.open_path(path)?.ok_or(FatError::PathNotFound)?;
        if !entry.is_dir() {
            return Err(FatError::NotADirectory);
        }

        self.list_dir_cluster(entry.first_cluster)
    }

    /// Lit un fichier à partir de son chemin absolu.
    ///
    /// Retourne:
    /// - `Ok(Some(bytes))` si le fichier existe
    /// - `Ok(None)` si le chemin n’existe pas
    /// - `Err(NotAFile)` si le chemin pointe vers un répertoire
    pub fn read_file_by_path(&self, path: &str) -> Result<Option<Vec<u8>>, FatError> {
        let entry = match self.open_path(path)? {
            Some(e) => e,
            None => return Ok(None),
        };

        if !entry.is_file() {
            return Err(FatError::NotAFile);
        }

        Ok(Some(self.read_file(&entry)?))
    }

    /// Résout un chemin absolu en une entrée de répertoire.
    ///
    /// - Le chemin doit commencer par `/`
    /// - la recherche est case-insensitive sur les noms courts (8.3),
    ///   parce qu’on normalise en majuscule
    pub fn open_path(&self, path: &str) -> Result<Option<DirEntry>, FatError> {
        if !path.starts_with('/') {
            return Err(FatError::Other);
        }
        if path == "/" {
            return Ok(None);
        }

        let mut current_cluster = self.root_cluster;
        let mut last_entry: Option<DirEntry> = None;

        for part in path.split('/').filter(|s| !s.is_empty()) {
            let target = normalize_name(part);
            let entries = self.list_dir_cluster(current_cluster)?;

            let mut found = None;
            for e in entries {
                if normalize_name(&e.name) == target {
                    current_cluster = e.first_cluster;
                    found = Some(e);
                    break;
                }
            }

            match found {
                Some(e) => last_entry = Some(e),
                None => return Ok(None),
            }
        }

        Ok(last_entry)
    }

    /// Lit un fichier à partir d’une entrée (`DirEntry`).
    ///
    /// On suit la chaîne de clusters dans la FAT, puis on reconstruit les octets
    /// jusqu’à `entry.size`.
    pub fn read_file(&self, entry: &DirEntry) -> Result<Vec<u8>, FatError> {
        if !entry.is_file() {
            return Err(FatError::NotAFile);
        }

        let mut remaining = entry.size as usize;
        if remaining == 0 {
            return Ok(Vec::new());
        }

        if entry.first_cluster < 2 {
            // pour un fichier non vide, un cluster < 2 est incohérent
            return Err(FatError::InvalidCluster);
        }

        let cluster_size = self.cluster_size();
        let mut out = Vec::with_capacity(remaining);

        let chain = self.follow_chain(entry.first_cluster, 4096)?;
        for cl in chain {
            let cluster = self.read_cluster(cl)?;
            let take = core::cmp::min(remaining, cluster_size);

            out.extend_from_slice(&cluster[..take]);
            remaining -= take;

            if remaining == 0 {
                break;
            }
        }

        Ok(out)
    }

    // ---------- internes (lecture) ----------

    fn bytes_per_sector(&self) -> usize {
        self.bytes_per_sector as usize
    }

    fn cluster_size(&self) -> usize {
        self.bytes_per_sector() * self.sectors_per_cluster as usize
    }

    fn fat_start_byte(&self) -> usize {
        self.reserved_sectors as usize * self.bytes_per_sector()
    }

    fn data_start_byte(&self) -> usize {
        self.fat_start_byte()
            + (self.num_fats as usize * self.sectors_per_fat as usize) * self.bytes_per_sector()
    }

    fn cluster_to_offset(&self, cluster: u32) -> Result<usize, FatError> {
        if cluster < 2 {
            return Err(FatError::InvalidCluster);
        }

        let index = (cluster - 2) as usize;
        let offset = self.data_start_byte() + index * self.cluster_size();

        if offset >= self.disk.len() {
            return Err(FatError::OutOfBounds);
        }

        Ok(offset)
    }

    fn read_cluster(&self, cluster: u32) -> Result<&[u8], FatError> {
        let offset = self.cluster_to_offset(cluster)?;
        let size = self.cluster_size();

        if offset + size > self.disk.len() {
            return Err(FatError::OutOfBounds);
        }

        Ok(&self.disk[offset..offset + size])
    }

    fn read_fat_entry(&self, cluster: u32) -> Result<u32, FatError> {
        let fat_start = self.fat_start_byte();
        let entry_offset = fat_start + cluster as usize * 4;

        if entry_offset + 4 > self.disk.len() {
            return Err(FatError::OutOfBounds);
        }

        let bytes = &self.disk[entry_offset..entry_offset + 4];
        let val = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);

        Ok(val & 0x0FFF_FFFF)
    }

    fn follow_chain(&self, start_cluster: u32, max_clusters: usize) -> Result<Vec<u32>, FatError> {
        if start_cluster < 2 {
            return Err(FatError::InvalidCluster);
        }

        let mut chain = Vec::new();
        let mut current = start_cluster;

        for _ in 0..max_clusters {
            chain.push(current);

            let next = self.read_fat_entry(current)?;
            if next >= 0x0FFF_FFF8 {
                break;
            }

            if next < 2 {
                return Err(FatError::InvalidCluster);
            }
            current = next;
        }

        Ok(chain)
    }

    fn list_dir_cluster(&self, start_cluster: u32) -> Result<Vec<DirEntry>, FatError> {
        let mut entries = Vec::new();
        let chain = self.follow_chain(start_cluster, 4096)?;

        let mut end_seen = false;

        for cl in chain {
            if end_seen {
                break;
            }

            let data = self.read_cluster(cl)?;
            for chunk in data.chunks(32) {
                if chunk.len() < 32 {
                    break;
                }

                // 0x00 = fin de répertoire (à partir de là, tout est libre)
                if chunk[0] == 0x00 {
                    end_seen = true;
                    break;
                }

                if let Some(e) = DirEntry::parse(chunk) {
                    entries.push(e);
                }
            }
        }

        Ok(entries)
    }
}

impl<'a> Fat32Mut<'a> {
    /// Construit une vue FAT32 depuis un dump en mémoire (lecture/écriture).
    pub fn new(disk: &'a mut [u8]) -> Result<Self, FatError> {
        let p = parse_bpb(&*disk)?;
        Ok(Self {
            disk,
            bytes_per_sector: p.bytes_per_sector,
            sectors_per_cluster: p.sectors_per_cluster,
            reserved_sectors: p.reserved_sectors,
            num_fats: p.num_fats,
            sectors_per_fat: p.sectors_per_fat,
            root_cluster: p.root_cluster,
        })
    }

    /// Donne une vue lecture seule sur le même buffer.
    ///
    /// Ça permet de réutiliser `open_path` / `list_root` sans dupliquer la logique.
    pub fn as_read(&self) -> Fat32<'_> {
        Fat32 {
            disk: &*self.disk,
            bytes_per_sector: self.bytes_per_sector,
            sectors_per_cluster: self.sectors_per_cluster,
            reserved_sectors: self.reserved_sectors,
            num_fats: self.num_fats,
            sectors_per_fat: self.sectors_per_fat,
            root_cluster: self.root_cluster,
        }
    }

    /// Écrit un fichier (création ou overwrite) dans l’image FAT32.
    ///
    /// Règles simples (volontaires) :
    /// - `path` doit être absolu et viser un fichier (pas un répertoire)
    /// - nom court 8.3 uniquement (ex: `HELLO.TXT`, `A.TXT`, `FILE`)
    /// - le répertoire parent doit exister
    ///
    /// Comportement :
    /// - si le fichier existe, on libère son ancienne chaîne de clusters
    /// - puis on alloue une nouvelle chaîne, on écrit les données, et on met à jour l’entrée
    /// - si `content` est vide, on crée un fichier vide (cluster = 0)
    pub fn write_file_by_path(&mut self, path: &str, content: &[u8]) -> Result<(), FatError> {
        if !path.starts_with('/') || path == "/" {
            return Err(FatError::Other);
        }

        let (parent_path, file_name) = split_parent(path)?;
        let (name_raw, ext_raw) = encode_short_name_8_3(file_name)?;

        let parent_cluster = if parent_path == "/" {
            self.root_cluster
        } else {
            let entry = self
                .as_read()
                .open_path(parent_path)?
                .ok_or(FatError::PathNotFound)?;
            if !entry.is_dir() {
                return Err(FatError::NotADirectory);
            }
            entry.first_cluster
        };

        let (existing_off, existing_entry) =
            self.find_dir_entry_offset_by_short_name(parent_cluster, &name_raw, &ext_raw)?;

        // Overwrite: on libère l’ancienne chaîne
        if let Some(e) = existing_entry.as_ref() {
            if e.is_dir() {
                return Err(FatError::NotAFile);
            }
            if e.first_cluster >= 2 {
                self.free_chain(e.first_cluster)?;
            }
        }

        // Allocation des clusters nécessaires
        let first_cluster = if content.is_empty() {
            0u32
        } else {
            let needed = div_ceil(content.len(), self.cluster_size());
            let chain = self.alloc_chain(needed)?;
            self.write_chain_data(&chain, content)?;
            chain[0]
        };

        // Écriture / mise à jour de l’entrée de répertoire
        let size = content.len() as u32;

        match existing_off {
            Some(off) => {
                self.write_dir_entry_at_offset(off, &name_raw, &ext_raw, first_cluster, size)?;
            }
            None => {
                let (free_off, was_end_marker, entry_end_in_disk) =
                    self.find_free_dir_entry_slot(parent_cluster)?;
                self.write_dir_entry_at_offset(
                    free_off,
                    &name_raw,
                    &ext_raw,
                    first_cluster,
                    size,
                )?;

                // Si on a remplacé un 0x00 (end-of-dir), on remet un 0x00 juste après
                // (si ça rentre dans le buffer). Ça garde un répertoire “propre”.
                if was_end_marker {
                    let next = free_off + 32;
                    if next < entry_end_in_disk {
                        self.disk[next] = 0x00;
                    }
                }
            }
        }

        Ok(())
    }

    // ---------- internes (écriture) ----------

    fn bytes_per_sector(&self) -> usize {
        self.bytes_per_sector as usize
    }

    fn cluster_size(&self) -> usize {
        self.bytes_per_sector() * self.sectors_per_cluster as usize
    }

    fn fat_start_byte(&self) -> usize {
        self.reserved_sectors as usize * self.bytes_per_sector()
    }

    fn fat_bytes_len(&self) -> usize {
        self.sectors_per_fat as usize * self.bytes_per_sector()
    }

    fn data_start_byte(&self) -> usize {
        self.fat_start_byte()
            + (self.num_fats as usize * self.sectors_per_fat as usize) * self.bytes_per_sector()
    }

    /// Dernier cluster valide, borné à la fois par:
    /// - la taille de la zone data
    /// - le nombre d’entrées disponibles dans la FAT
    fn max_cluster_number(&self) -> Result<u32, FatError> {
        let data_start = self.data_start_byte();
        if data_start >= self.disk.len() {
            return Err(FatError::OutOfBounds);
        }

        let cs = self.cluster_size();
        if cs == 0 {
            return Err(FatError::NotFat32);
        }

        let data_len = self.disk.len() - data_start;
        let data_clusters = (data_len / cs) as u32;
        if data_clusters == 0 {
            return Err(FatError::NotFat32);
        }
        let last_by_data = 2 + data_clusters - 1;

        let fat_entries = (self.fat_bytes_len() / 4) as u32;
        if fat_entries < 3 {
            return Err(FatError::NotFat32);
        }
        let last_by_fat = fat_entries - 1;

        Ok(core::cmp::min(last_by_data, last_by_fat))
    }

    fn cluster_to_offset(&self, cluster: u32) -> Result<usize, FatError> {
        if cluster < 2 {
            return Err(FatError::InvalidCluster);
        }
        let index = (cluster - 2) as usize;
        let offset = self.data_start_byte() + index * self.cluster_size();
        if offset >= self.disk.len() {
            return Err(FatError::OutOfBounds);
        }
        Ok(offset)
    }

    fn read_fat_entry(&self, cluster: u32) -> Result<u32, FatError> {
        let fat_start = self.fat_start_byte();
        let entry_offset = fat_start + cluster as usize * 4;
        if entry_offset + 4 > self.disk.len() {
            return Err(FatError::OutOfBounds);
        }
        let bytes = &self.disk[entry_offset..entry_offset + 4];
        let val = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        Ok(val & 0x0FFF_FFFF)
    }

    fn write_fat_entry_all(&mut self, cluster: u32, value: u32) -> Result<(), FatError> {
        let val = value & 0x0FFF_FFFF;
        let bytes = val.to_le_bytes();

        let fat0 = self.fat_start_byte();
        let fat_len = self.fat_bytes_len();

        for i in 0..self.num_fats as usize {
            let base = fat0 + i * fat_len;
            let off = base + cluster as usize * 4;
            if off + 4 > self.disk.len() {
                return Err(FatError::OutOfBounds);
            }
            self.disk[off..off + 4].copy_from_slice(&bytes);
        }

        Ok(())
    }

    fn follow_chain(&self, start_cluster: u32, max_clusters: usize) -> Result<Vec<u32>, FatError> {
        if start_cluster < 2 {
            return Err(FatError::InvalidCluster);
        }

        let mut result = Vec::new();
        let mut current = start_cluster;

        for _ in 0..max_clusters {
            result.push(current);
            let next = self.read_fat_entry(current)?;
            if next >= 0x0FFF_FFF8 {
                break;
            }
            if next < 2 {
                return Err(FatError::InvalidCluster);
            }
            current = next;
        }

        Ok(result)
    }

    fn free_chain(&mut self, start_cluster: u32) -> Result<(), FatError> {
        if start_cluster < 2 {
            return Ok(());
        }
        let chain = self.follow_chain(start_cluster, 4096)?;
        for cl in chain {
            self.write_fat_entry_all(cl, 0)?;
        }
        Ok(())
    }

    fn alloc_chain(&mut self, needed: usize) -> Result<Vec<u32>, FatError> {
        if needed == 0 {
            return Ok(Vec::new());
        }

        let max_cl = self.max_cluster_number()?;
        let mut found = Vec::with_capacity(needed);

        // Scan simple : cluster libre = entrée FAT == 0
        for cl in 2..=max_cl {
            if self.read_fat_entry(cl)? == 0 {
                found.push(cl);
                if found.len() == needed {
                    break;
                }
            }
        }

        if found.len() != needed {
            return Err(FatError::NoSpaceLeft);
        }

        // Chaînage : cl[i] -> cl[i+1], dernier -> EOC
        for i in 0..found.len() {
            let v = if i + 1 < found.len() { found[i + 1] } else { FAT32_EOC };
            self.write_fat_entry_all(found[i], v)?;
        }

        Ok(found)
    }

    fn write_chain_data(&mut self, chain: &[u32], content: &[u8]) -> Result<(), FatError> {
        let cs = self.cluster_size();
        let mut pos = 0usize;

        for &cl in chain {
            let off = self.cluster_to_offset(cl)?;
            if off + cs > self.disk.len() {
                return Err(FatError::OutOfBounds);
            }

            let end = core::cmp::min(pos + cs, content.len());
            let chunk = &content[pos..end];

            self.disk[off..off + chunk.len()].copy_from_slice(chunk);

            // Nettoyage du reste du cluster (c’est plus propre pour les tests et pour “cat”)
            for b in &mut self.disk[off + chunk.len()..off + cs] {
                *b = 0;
            }

            pos = end;
            if pos >= content.len() {
                break;
            }
        }

        Ok(())
    }

    fn find_dir_entry_offset_by_short_name(
        &self,
        dir_cluster: u32,
        name_raw: &[u8; 8],
        ext_raw: &[u8; 3],
    ) -> Result<(Option<usize>, Option<DirEntry>), FatError> {
        let cs = self.cluster_size();
        let chain = self.follow_chain(dir_cluster, 4096)?;

        for &cl in &chain {
            let off = self.cluster_to_offset(cl)?;
            if off + cs > self.disk.len() {
                return Err(FatError::OutOfBounds);
            }

            let data = &self.disk[off..off + cs];
            for (i, chunk) in data.chunks(32).enumerate() {
                if chunk.len() < 32 {
                    break;
                }
                if chunk[0] == 0x00 {
                    // fin de répertoire
                    return Ok((None, None));
                }
                if chunk[0] == 0xE5 {
                    continue;
                }

                // match strict sur les octets 8.3
                if &chunk[0..8] == &name_raw[..] && &chunk[8..11] == &ext_raw[..] {
                    let abs_off = off + i * 32;
                    let parsed = DirEntry::parse(chunk);
                    return Ok((Some(abs_off), parsed));
                }
            }
        }

        Ok((None, None))
    }

    /// Trouve un slot libre dans un répertoire.
    ///
    /// Retourne:
    /// - l’offset dans `disk`
    /// - `was_end_marker`: vrai si c’était un `0x00` (fin de répertoire)
    /// - `entry_end_in_disk`: limite du cluster dans le buffer (pour écrire un 0x00 après)
    fn find_free_dir_entry_slot(&self, dir_cluster: u32) -> Result<(usize, bool, usize), FatError> {
        let cs = self.cluster_size();
        let chain = self.follow_chain(dir_cluster, 4096)?;

        for &cl in &chain {
            let off = self.cluster_to_offset(cl)?;
            let cluster_end = off + cs;

            if cluster_end > self.disk.len() {
                return Err(FatError::OutOfBounds);
            }

            let data = &self.disk[off..cluster_end];
            for (i, chunk) in data.chunks(32).enumerate() {
                if chunk.len() < 32 {
                    break;
                }

                if chunk[0] == 0x00 {
                    return Ok((off + i * 32, true, cluster_end));
                }
                if chunk[0] == 0xE5 {
                    return Ok((off + i * 32, false, cluster_end));
                }
            }
        }

        // Version simple: on n’alloue pas de nouveau cluster de répertoire.
        Err(FatError::NoSpaceLeft)
    }

    fn write_dir_entry_at_offset(
        &mut self,
        offset: usize,
        name_raw: &[u8; 8],
        ext_raw: &[u8; 3],
        first_cluster: u32,
        size: u32,
    ) -> Result<(), FatError> {
        if offset + 32 > self.disk.len() {
            return Err(FatError::OutOfBounds);
        }

        let hi = ((first_cluster >> 16) as u16).to_le_bytes();
        let lo = ((first_cluster & 0xFFFF) as u16).to_le_bytes();
        let size_bytes = size.to_le_bytes();

        let e = &mut self.disk[offset..offset + 32];

        // Name + ext
        e[0..8].copy_from_slice(name_raw);
        e[8..11].copy_from_slice(ext_raw);

        // Attributs : archive (fichier)
        e[11] = 0x20;

        // Champs “date/heure” et divers : on met à zéro (écriture simple)
        for b in &mut e[12..20] {
            *b = 0;
        }

        // First cluster high
        e[20] = hi[0];
        e[21] = hi[1];

        for b in &mut e[22..26] {
            *b = 0;
        }

        // First cluster low
        e[26] = lo[0];
        e[27] = lo[1];

        // Size
        e[28..32].copy_from_slice(&size_bytes);

        Ok(())
    }
}

// ---------- helpers BPB + path + nom 8.3 ----------

#[derive(Clone, Copy)]
/// Paramètres du BPB nécessaires pour naviguer dans le volume
///
/// On ne lit que ce qui sert à calculer les offsets et tailles comme par exemple : 
/// taille de secteur, taille de cluster, FAT, cluster racine...
struct BpbParams {
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    sectors_per_fat: u32,
    root_cluster: u32,
}

/// Parse le BPB du secteur 0 et extrait les paramètres utiles.
///
/// Effectue des vérifications minimales pour éviter un état incohérent.
fn parse_bpb(disk: &[u8]) -> Result<BpbParams, FatError> {
    if disk.len() < 512 {
        return Err(FatError::BufferTooSmall);
    }

    let b = &disk[0..512];

    let bytes_per_sector = u16::from_le_bytes([b[11], b[12]]);
    let sectors_per_cluster = b[13];
    let reserved_sectors = u16::from_le_bytes([b[14], b[15]]);
    let num_fats = b[16];
    let sectors_per_fat = u32::from_le_bytes([b[36], b[37], b[38], b[39]]);
    let root_cluster = u32::from_le_bytes([b[44], b[45], b[46], b[47]]);

    // Checks minimalistes pour éviter un état incohérent
    if bytes_per_sector == 0 || sectors_per_cluster == 0 || num_fats == 0 {
        return Err(FatError::NotFat32);
    }
    if sectors_per_fat == 0 {
        return Err(FatError::NotFat32);
    }

    Ok(BpbParams {
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        num_fats,
        sectors_per_fat,
        root_cluster,
    })
}

/// Normalise un nom pour comparer facilement (on passe en majuscule).
fn normalize_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        out.push(ch.to_ascii_uppercase());
    }
    out
}

/// Découpe `"/A/B/C.TXT"` en (`"/A/B"`, `"C.TXT"`).
fn split_parent(path: &str) -> Result<(&str, &str), FatError> {
    let path = path.trim_end_matches('/');
    if path == "/" {
        return Err(FatError::Other);
    }

    let idx = path.rfind('/').ok_or(FatError::Other)?;
    let parent = if idx == 0 { "/" } else { &path[..idx] };
    let name = &path[idx + 1..];

    if name.is_empty() {
        return Err(FatError::Other);
    }
    Ok((parent, name))
}

/// Encode un nom en format court 8.3.
///
/// Exemples :
/// - `"HELLO.TXT"` -> name=`"HELLO   "`, ext=`"TXT"`
/// - `"DIR"`       -> name=`"DIR     "`, ext=`"   "`
///
/// Limites volontaires :
/// - ASCII uniquement
/// - 1 point max (séparateur extension)
/// - base <= 8, ext <= 3
/// - pas de `.` dans la base ou l’extension
fn encode_short_name_8_3(name: &str) -> Result<([u8; 8], [u8; 3]), FatError> {
    let mut base = name;
    let mut ext = "";

    if let Some(dot) = name.rfind('.') {
        base = &name[..dot];
        ext = &name[dot + 1..];
    }

    if base.is_empty() || base.len() > 8 || ext.len() > 3 {
        return Err(FatError::InvalidName);
    }
    if base.contains('.') || ext.contains('.') {
        return Err(FatError::InvalidName);
    }

    let mut n = [b' '; 8];
    let mut e = [b' '; 3];

    for (i, ch) in base.bytes().enumerate() {
        if !ch.is_ascii() || ch == b'/' {
            return Err(FatError::InvalidName);
        }
        n[i] = ch.to_ascii_uppercase();
    }

    for (i, ch) in ext.bytes().enumerate() {
        if !ch.is_ascii() || ch == b'/' {
            return Err(FatError::InvalidName);
        }
        e[i] = ch.to_ascii_uppercase();
    }

    Ok((n, e))
}

/// Division entière avec arrondi vers le haut.
fn div_ceil(a: usize, b: usize) -> usize {
    if b == 0 {
        0
    } else {
        (a + b - 1) / b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mini volume FAT32 en mémoire :
    /// - 1 secteur BPB
    /// - 1 secteur FAT
    /// - data cluster (sector) 2 = racine : HELLO.TXT + DIR
    /// - data cluster (sector) 3 = contenu "HELLO"
    /// - data cluster (sector) 4 = répertoire DIR (vide)
    ///
    /// Les clusters 5.. sont libres, ce qui permet de tester l’écriture.
    fn build_test_image() -> [u8; 5120] {
        const SECTOR_SIZE: usize = 512;
        const NUM_SECTORS: usize = 10;
        let mut disk = [0u8; SECTOR_SIZE * NUM_SECTORS];

        // BPB
        {
            let b = &mut disk[0..SECTOR_SIZE];

            // bytes_per_sector = 512
            b[11] = 0x00;
            b[12] = 0x02;

            // sectors_per_cluster = 1
            b[13] = 0x01;

            // reserved_sectors = 1
            b[14] = 0x01;
            b[15] = 0x00;

            // num_fats = 1
            b[16] = 0x01;

            // sectors_per_fat = 1
            b[36] = 0x01;
            b[37] = 0x00;
            b[38] = 0x00;
            b[39] = 0x00;

            // root_cluster = 2
            b[44] = 0x02;
            b[45] = 0x00;
            b[46] = 0x00;
            b[47] = 0x00;
        }

        // FAT (secteur 1)
        {
            let fat_start = SECTOR_SIZE;
            let fat = &mut disk[fat_start..fat_start + SECTOR_SIZE];

            let eoc_bytes = FAT32_EOC.to_le_bytes();

            // cluster 2 (root) -> EOC
            fat[2 * 4..2 * 4 + 4].copy_from_slice(&eoc_bytes);
            // cluster 3 (HELLO.TXT) -> EOC
            fat[3 * 4..3 * 4 + 4].copy_from_slice(&eoc_bytes);
            // cluster 4 (DIR) -> EOC
            fat[4 * 4..4 * 4 + 4].copy_from_slice(&eoc_bytes);

            // clusters 5.. = 0 -> libres
        }

        // root dir = cluster 2 -> secteur 2
        {
            let root_off = 2 * SECTOR_SIZE;
            let dir = &mut disk[root_off..root_off + SECTOR_SIZE];

            // HELLO.TXT
            let mut hello = [0u8; 32];
            hello[0..8].copy_from_slice(b"HELLO   ");
            hello[8..11].copy_from_slice(b"TXT");
            hello[11] = 0x20; // archive file
            // first_cluster = 3
            hello[26] = 0x03;
            hello[27] = 0x00;
            // size = 5
            hello[28] = 5;
            dir[0..32].copy_from_slice(&hello);

            // DIR
            let mut subdir = [0u8; 32];
            subdir[0..8].copy_from_slice(b"DIR     ");
            subdir[8..11].copy_from_slice(b"   ");
            subdir[11] = 0x10; // directory
            // first_cluster = 4
            subdir[26] = 0x04;
            subdir[27] = 0x00;
            dir[32..64].copy_from_slice(&subdir);

            // end of dir
            dir[64] = 0x00;
        }

        // cluster 3 data -> secteur 3
        {
            let off = 3 * SECTOR_SIZE;
            disk[off..off + 5].copy_from_slice(b"HELLO");
        }

        // cluster 4 data -> secteur 4 (DIR empty)
        {
            let off = 4 * SECTOR_SIZE;
            disk[off] = 0x00;
        }

        disk
    }

    fn fat_entry_raw(disk: &[u8], cluster: u32) -> u32 {
        // Dans notre image de test: reserved=1, bytes_per_sector=512 donc FAT start = 512.
        let fat_start = 512usize;
        let off = fat_start + cluster as usize * 4;
        let bytes = &disk[off..off + 4];
        let v = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        v & 0x0FFF_FFFF
    }

    #[test]
    fn new_on_too_small_buffer_fails() {
        let tiny = [0u8; 128];
        let err = Fat32::new(&tiny).unwrap_err();
        assert_eq!(err, FatError::BufferTooSmall);
    }

    #[test]
    fn list_root_and_read_file() {
        let disk = build_test_image();
        let fs = Fat32::new(&disk).expect("Fat32::new failed");

        let root = fs.list_root().expect("list_root failed");
        assert_eq!(root.len(), 2);

        let hello = root
            .iter()
            .find(|e| e.name == "HELLO.TXT")
            .expect("HELLO.TXT missing");
        let dir = root.iter().find(|e| e.name == "DIR").expect("DIR missing");

        assert!(hello.is_file());
        assert!(dir.is_dir());

        let content = fs.read_file_by_path("/HELLO.TXT").unwrap().unwrap();
        assert_eq!(content, b"HELLO");
    }

    #[test]
    fn list_dir_on_file_returns_not_a_directory() {
        let disk = build_test_image();
        let fs = Fat32::new(&disk).unwrap();

        let err = fs.list_dir_path("/HELLO.TXT").unwrap_err();
        assert_eq!(err, FatError::NotADirectory);
    }

    #[test]
    fn read_file_on_directory_via_open_path_returns_not_a_file() {
        let disk = build_test_image();
        let fs = Fat32::new(&disk).unwrap();

        let entry = fs.open_path("/DIR").unwrap().unwrap();
        assert!(entry.is_dir());

        let err = fs.read_file(&entry).unwrap_err();
        assert_eq!(err, FatError::NotAFile);
    }

    #[test]
    fn open_path_is_case_insensitive_for_short_names() {
        let disk = build_test_image();
        let fs = Fat32::new(&disk).unwrap();

        let entry = fs.open_path("/hello.txt").unwrap().unwrap();
        assert_eq!(entry.name, "HELLO.TXT");
        assert!(entry.is_file());
    }

    #[test]
    fn write_create_new_file_and_read_back() {
        let mut disk = build_test_image();

        {
            let mut rw = Fat32Mut::new(&mut disk).unwrap();
            rw.write_file_by_path("/NEW.TXT", b"ABC").unwrap();
        }

        let ro = Fat32::new(&disk).unwrap();
        let content = ro.read_file_by_path("/NEW.TXT").unwrap().unwrap();
        assert_eq!(content, b"ABC");
    }

    #[test]
    fn write_overwrite_existing_file() {
        let mut disk = build_test_image();

        {
            let mut rw = Fat32Mut::new(&mut disk).unwrap();
            rw.write_file_by_path("/HELLO.TXT", b"HELLO WORLD").unwrap();
        }

        let ro = Fat32::new(&disk).unwrap();
        let content = ro.read_file_by_path("/HELLO.TXT").unwrap().unwrap();
        assert_eq!(content, b"HELLO WORLD");
    }

    #[test]
    fn write_rejects_invalid_8_3_name() {
        let mut disk = build_test_image();

        let res = {
            let mut rw = Fat32Mut::new(&mut disk).unwrap();
            rw.write_file_by_path("/TOO_LONG_NAME.TXT", b"x")
        };

        assert_eq!(res.unwrap_err(), FatError::InvalidName);
    }

    #[test]
    fn write_fails_when_parent_directory_missing() {
        let mut disk = build_test_image();

        let res = {
            let mut rw = Fat32Mut::new(&mut disk).unwrap();
            rw.write_file_by_path("/NOPE/FILE.TXT", b"x")
        };

        assert_eq!(res.unwrap_err(), FatError::PathNotFound);
    }

    #[test]
    fn overwrite_frees_old_clusters_in_fat() {
        let mut disk = build_test_image();

        // 600 bytes => 2 clusters (cluster_size=512)
        let big = vec![0x41u8; 600];

        let (first_cluster, second_cluster) = {
            let mut rw = Fat32Mut::new(&mut disk).unwrap();
            rw.write_file_by_path("/BIG.TXT", &big).unwrap();

            let ro = rw.as_read();
            let e = ro.open_path("/BIG.TXT").unwrap().unwrap();
            assert_eq!(e.size as usize, big.len());
            assert!(e.first_cluster >= 2);

            let c1 = e.first_cluster;
            let c2 = fat_entry_raw(&disk, c1);
            assert!(c2 >= 2, "le fichier BIG.TXT devrait chaîner sur un 2e cluster");
            assert!(fat_entry_raw(&disk, c2) >= 0x0FFF_FFF8, "le 2e cluster devrait être EOC");

            (c1, c2)
        };

        // Overwrite en fichier vide => doit libérer les anciens clusters
        {
            let mut rw = Fat32Mut::new(&mut disk).unwrap();
            rw.write_file_by_path("/BIG.TXT", b"").unwrap();
        }

        // FAT entries doivent être à 0 (libres)
        assert_eq!(fat_entry_raw(&disk, first_cluster), 0);
        assert_eq!(fat_entry_raw(&disk, second_cluster), 0);

        let ro = Fat32::new(&disk).unwrap();
        let e = ro.open_path("/BIG.TXT").unwrap().unwrap();
        assert_eq!(e.size, 0);
        assert_eq!(e.first_cluster, 0);
    }
}
