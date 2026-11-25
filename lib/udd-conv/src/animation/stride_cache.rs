//! The stride cache: strides measured on earlier packages (`udd_assets::animation::mobile_strides`), keyed by package
//! kind, body, action and the [`stride_source_hash`](udd_assets::animation::mobile_strides::stride_source_hash) of the
//! client frames the clip was made from, shipped with the conversion tables as `MobileStrides.tsv`
//! ([`crate::conversion_tables::MOBILE_STRIDES_TSV`]).
//!
//! The packers fill a new package's clips from it ([`super::mobile_strides::keep_repacked_strides`]), so an unmodified
//! client needs no measuring: like a repack's carry-over, a stride follows its clip's client frames through any frame
//! filter, trim, scale, pixel format or compression, none of which moves a foot. A modified client's clips hash
//! otherwise and miss. `udd-cli tool strides cache-export` writes it from measured packages, merging the tables of
//! several client versions.
//!
//! The table is tab-separated under a fixed header, the strides at full `f32` precision so that a package filled from
//! it holds the very values measured. It is generated and read strictly: an unknown column, a malformed field or a
//! key given twice is an error naming the line.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use log::info;
use udd_assets::animation::mobile_strides::{
    MobileStrideRecord, StrideSource, StrideSourceRecord, STRIDE_FACING_COUNT,
};
use uocf::utils::bundled_files;

use crate::conversion_tables::MOBILE_STRIDES_TSV;

/// The file name of the shipped table.
pub const STRIDE_CACHE_FILE_NAME: &str = "MobileStrides.tsv";

/// The columns, in order: the package kind, body, action, the side facings' strides, the stride, the source hash.
const COLUMNS: [&str; 8] = ["source", "body", "action", "south", "south-west", "west", "stride", "source-hash"];
const SEPARATOR: char = '\t';
const NONE_FIELD: &str = "-";
const COMMENT_PREFIX: char = '#';
const HASH_HEX_DIGITS: usize = 16;
/// The comment lines the table starts with.
const HEADER_COMMENTS: &[&str] = &[
    "The mobiles' measured walk and run strides, by the client frames they were measured on. The packers",
    "(udd-cli pack-mobile-anims and pack-ec-mobile-anims, the converter GUI) give a new package's clip the stride of",
    "its row here when the clip's client frames hash the same, so an unmodified client needs no measuring.",
    "Generated, do not edit: `udd-cli tool strides cache-export PACKAGE... [--ccdir DIR] [--ecdir DIR] [--merge TABLE]`",
    "over packages measured with `udd-cli tool strides measure full`, merging the tables of several client versions.",
    "Columns, tab-separated: source, the package (cc Classic, ec Enhanced); body, the server body; action; south,",
    "south-west and west, the stride measured on that facing, and stride, their median, which paces the clip; all in",
    "side tiles a cycle at full precision, `-` where none measured; source-hash, the XXH64 of the client frames the",
    "clip was made from (metadata/stride_sources.bin, docs/dev_wiki/UDDP_FORMATS.md 3.1), 16 hex digits.",
];

/// What a stride is cached by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StrideCacheKey {
    pub source: StrideSource,
    pub body_id: u32,
    pub action_id: u16,
    /// The `source_hash` of the clip's [`StrideSourceRecord`].
    pub source_hash: u64,
}

/// A clip's measurement, as its [`MobileStrideRecord`] holds it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CachedStride {
    pub facing_tiles: [Option<f32>; STRIDE_FACING_COUNT],
    pub stride_tiles: Option<f32>,
}

impl CachedStride {
    fn of(record: &MobileStrideRecord) -> Self {
        Self { facing_tiles: record.facing_tiles, stride_tiles: record.stride_tiles }
    }

    /// The same values to the bit: what a measurement gives again on the same frames.
    fn same(&self, other: &Self) -> bool {
        let bits = |tiles: Option<f32>| tiles.map(f32::to_bits);
        bits(self.stride_tiles) == bits(other.stride_tiles)
            && self.facing_tiles.iter().zip(&other.facing_tiles).all(|(&a, &b)| bits(a) == bits(b))
    }

    fn fields(&self) -> String {
        let tiles = |tiles: Option<f32>| tiles.map_or(NONE_FIELD.to_string(), |tiles| tiles.to_string());
        let mut fields = String::new();
        for facing in self.facing_tiles {
            write!(fields, "{SEPARATOR}{}", tiles(facing)).unwrap();
        }
        write!(fields, "{SEPARATOR}{}", tiles(self.stride_tiles)).unwrap();
        fields
    }
}

/// What a package took from the cache: the strides of its clips that hit, and how many of its clips missed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CacheFill {
    pub records: Vec<MobileStrideRecord>,
    pub missed: usize,
}

/// The cached strides, sorted by package kind, body, action and source hash.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StrideCache {
    entries: BTreeMap<StrideCacheKey, CachedStride>,
}

impl StrideCache {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, key: &StrideCacheKey) -> Option<&CachedStride> {
        self.entries.get(key)
    }

    /// Adds `stride` under `key`. A key held with other values is an error: the same client frames measured otherwise.
    pub fn insert(&mut self, key: StrideCacheKey, stride: CachedStride) -> eyre::Result<()> {
        match self.entries.get(&key) {
            Some(held) if !held.same(&stride) => eyre::bail!("{} body {} action {} source hash {:016x} is cached as \
                ({}) and as ({}): the same client frames measured otherwise", key.source.token(), key.body_id,
                key.action_id, key.source_hash, held.fields().trim(), stride.fields().trim()),
            Some(_) => Ok(()),
            None => {
                self.entries.insert(key, stride);
                Ok(())
            }
        }
    }

    /// Adds every stride of `other` ([`Self::insert`]).
    pub fn merge(&mut self, other: Self) -> eyre::Result<()> {
        other.entries.into_iter().try_for_each(|(key, stride)| self.insert(key, stride))
    }

    /// Adds the strides of a `source` package whose strides are `records` and whose stride clips are `sources`: those
    /// measured on the frames it has (their clip hash that of their clip's source record). Gives how many it added or
    /// held already.
    pub fn add_package(&mut self, source: StrideSource, records: &[MobileStrideRecord],
        sources: &[StrideSourceRecord]) -> eyre::Result<usize> {
        let by_clip: BTreeMap<(u32, u16), &StrideSourceRecord> =
            sources.iter().map(|record| ((record.body_id, record.action_id), record)).collect();
        let mut added = 0;
        for record in records {
            let Some(clip) = by_clip.get(&(record.body_id, record.action_id)) else { continue };
            if clip.clip_hash != record.clip_hash {
                continue;
            }
            self.insert(StrideCacheKey { source, body_id: record.body_id, action_id: record.action_id,
                source_hash: clip.source_hash }, CachedStride::of(record))?;
            added += 1;
        }
        Ok(added)
    }

    /// The strides of the clips of a `source` package, its stride clips `sources`, that `has_stride` says have none:
    /// each cached under its clip's source, with its clip hash in the package; the others missed.
    pub fn fill(&self, source: StrideSource, sources: &[StrideSourceRecord],
        has_stride: impl Fn(&StrideSourceRecord) -> bool) -> CacheFill {
        let mut fill = CacheFill::default();
        for clip in sources.iter().filter(|clip| !has_stride(clip)) {
            let key = StrideCacheKey { source, body_id: clip.body_id, action_id: clip.action_id,
                source_hash: clip.source_hash };
            match self.entries.get(&key) {
                Some(cached) => fill.records.push(MobileStrideRecord { body_id: clip.body_id,
                    action_id: clip.action_id, clip_hash: clip.clip_hash, stride_tiles: cached.stride_tiles,
                    facing_tiles: cached.facing_tiles }),
                None => fill.missed += 1,
            }
        }
        fill
    }

    /// The table: the comment header, the column line and a line per stride.
    pub fn format(&self) -> String {
        let mut text = String::new();
        for comment in HEADER_COMMENTS {
            writeln!(text, "{COMMENT_PREFIX} {comment}").unwrap();
        }
        writeln!(text, "{}", COLUMNS.join(&SEPARATOR.to_string())).unwrap();
        for (key, stride) in &self.entries {
            writeln!(text, "{}{SEPARATOR}{}{SEPARATOR}{}{}{SEPARATOR}{:0width$x}", key.source.token(), key.body_id,
                key.action_id, stride.fields(), key.source_hash, width = HASH_HEX_DIGITS).unwrap();
        }
        text
    }

    /// Reads a table strictly: `#` comment and blank lines aside, the column line, then a line per stride. Errors
    /// name `name` and the line.
    pub fn parse(name: &str, text: &str) -> eyre::Result<Self> {
        let mut lines = text.lines().enumerate()
            .filter(|(_, line)| !line.trim().is_empty() && !line.starts_with(COMMENT_PREFIX));
        let (header_index, header) = lines.next().ok_or_else(|| eyre::eyre!("{name}: no column line"))?;
        let header: Vec<&str> = header.split(SEPARATOR).collect();
        if header != COLUMNS {
            eyre::bail!("{name}:{}: the columns are {:?}, not {:?}", header_index + 1, COLUMNS.join(" "),
                header.join(" "));
        }
        let mut cache = Self::default();
        let mut lines_of: BTreeMap<StrideCacheKey, usize> = BTreeMap::new();
        for (index, line) in lines {
            let line_number = index + 1;
            let error = |what: &str| eyre::eyre!("{name}:{line_number}: {what}: {line:?}");
            let fields: Vec<&str> = line.split(SEPARATOR).collect();
            let [source, body, action, south, south_west, west, stride, source_hash] =
                <[&str; COLUMNS.len()]>::try_from(fields)
                    .map_err(|_| error(&format!("expected {} tab-separated fields", COLUMNS.len())))?;
            let tiles = |field: &str| -> eyre::Result<Option<f32>> {
                match field {
                    NONE_FIELD => Ok(None),
                    field => field.parse::<f32>().ok().filter(|tiles| tiles.is_finite()).map(Some)
                        .ok_or_else(|| error(&format!("`{field}` is not a number of tiles or `-`"))),
                }
            };
            let stride_tiles = tiles(stride)?;
            if stride_tiles.is_some_and(|tiles| tiles <= 0.0) {
                return Err(error("the stride is not positive"));
            }
            let is_hash = source_hash.len() == HASH_HEX_DIGITS
                && source_hash.bytes().all(|digit| digit.is_ascii_digit() || (b'a'..=b'f').contains(&digit));
            let key = StrideCacheKey {
                source: StrideSource::from_token(source).ok_or_else(|| error("the source is `cc` or `ec`"))?,
                body_id: body.parse().map_err(|_| error("the body is not a number"))?,
                action_id: action.parse().map_err(|_| error("the action is not a number"))?,
                source_hash: is_hash.then(|| u64::from_str_radix(source_hash, 16).ok()).flatten()
                    .ok_or_else(|| error("the source hash is not 16 lowercase hex digits"))?,
            };
            if let Some(first) = lines_of.insert(key, line_number) {
                return Err(error(&format!("the clip and source hash of line {first} again")));
            }
            cache.entries.insert(key, CachedStride { facing_tiles: [tiles(south)?, tiles(south_west)?, tiles(west)?],
                stride_tiles });
        }
        Ok(cache)
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        let text = std::fs::read_to_string(path).wrap_err_with(|| format!("read {}", path.display()))?;
        Self::parse(&path.display().to_string(), &text)
    }
}

/// Reads the stride cache at `given`, else the copy shipped with the tools; gives its path too.
pub fn load_stride_cache(given: Option<&Path>) -> eyre::Result<(PathBuf, StrideCache)> {
    let path = bundled_files::resolve(given, MOBILE_STRIDES_TSV)?;
    let cache = StrideCache::load(&path)?;
    Ok((path, cache))
}

/// [`load_stride_cache`] for a packer, which logs what it read.
pub(crate) fn load_packer_stride_cache(given: Option<&Path>) -> eyre::Result<StrideCache> {
    let (path, cache) = load_stride_cache(given)?;
    info!("Stride cache: {} strides from {}", cache.len(), path.display());
    Ok(cache)
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_assets::animation::mobile_strides::format_stride_table;

    fn record(body_id: u32, action_id: u16, clip_hash: u64, stride_tiles: Option<f32>) -> MobileStrideRecord {
        MobileStrideRecord { body_id, action_id, clip_hash, stride_tiles,
            facing_tiles: [stride_tiles.map(|tiles| tiles + 0.1), None, Some(1.0 / 3.0)] }
    }

    fn source(body_id: u32, action_id: u16, clip_hash: u64, source_hash: u64) -> StrideSourceRecord {
        StrideSourceRecord { body_id, action_id, clip_hash, source_hash }
    }

    /// A measured package's strides go into the cache and come back on a repack of the same client frames whose
    /// frames hash otherwise (another pixel format, filter or scale): the same records but for the clip hashes, the
    /// values to the bit, so the strides export is the same but for its hash column.
    #[test]
    fn a_cache_round_trip_reproduces_the_measured_strides() {
        let measured = [record(1, 0, 10, Some(1.445_678_9)), record(1, 1, 11, None), record(400, 0, 40, Some(0.7)),
            record(400, 2, 42, Some(2.0 / 3.0))];
        let packed = [source(1, 0, 10, 100), source(1, 1, 11, 101), source(400, 0, 40, 400), source(400, 2, 42, 402)];
        let mut cache = StrideCache::default();
        assert_eq!(cache.add_package(StrideSource::Classic, &measured, &packed).unwrap(), measured.len());

        let text = cache.format();
        let parsed = StrideCache::parse("round trip", &text).unwrap();
        assert_eq!(parsed, cache);
        assert_eq!(parsed.format(), text);

        // The same package packed again: every clip hits.
        let fill = parsed.fill(StrideSource::Classic, &packed, |_| false);
        assert_eq!((fill.records.as_slice(), fill.missed), (measured.as_slice(), 0));
        let comments = ["a test".to_string()];
        assert_eq!(format_stride_table(StrideSource::Classic, &fill.records, &comments),
            format_stride_table(StrideSource::Classic, &measured, &comments));

        // Packed otherwise from the same client frames: the strides take the new clip hashes.
        let repacked: Vec<StrideSourceRecord> = packed.iter()
            .map(|clip| StrideSourceRecord { clip_hash: clip.clip_hash + 1000, ..*clip }).collect();
        let fill = parsed.fill(StrideSource::Classic, &repacked, |_| false);
        let expected: Vec<MobileStrideRecord> = measured.iter()
            .map(|record| MobileStrideRecord { clip_hash: record.clip_hash + 1000, ..*record }).collect();
        assert_eq!(fill.records, expected);
    }

    /// A clip whose client frames differ, of the other package kind, or that has a stride already, takes nothing.
    #[test]
    fn a_fill_misses_other_client_frames_and_skips_clips_with_a_stride() {
        let mut cache = StrideCache::default();
        cache.add_package(StrideSource::Enhanced, &[record(9, 0, 90, Some(1.5)), record(9, 1, 91, Some(2.5))],
            &[source(9, 0, 90, 900), source(9, 1, 91, 901)]).unwrap();
        let now = [source(9, 0, 95, 900), source(9, 1, 91, 999), source(10, 0, 100, 1000)];
        let fill = cache.fill(StrideSource::Enhanced, &now, |_| false);
        assert_eq!(fill.records, vec![MobileStrideRecord { clip_hash: 95, ..record(9, 0, 90, Some(1.5)) }]);
        assert_eq!(fill.missed, 2);
        assert_eq!(cache.fill(StrideSource::Classic, &now, |_| false).missed, now.len());
        let fill = cache.fill(StrideSource::Enhanced, &now, |clip| clip.body_id == 9 && clip.action_id == 0);
        assert_eq!((fill.records.len(), fill.missed), (0, 2));
    }

    /// Only strides measured on the frames the package has go in; tables of several client versions merge where they
    /// agree and fail where the same client frames measured otherwise.
    #[test]
    fn packages_and_tables_merge_only_where_they_agree() {
        let mut cache = StrideCache::default();
        // Body 2's stride was measured on other frames than the package has now.
        let added = cache.add_package(StrideSource::Enhanced, &[record(1, 0, 10, Some(1.0)), record(2, 0, 20, Some(1.0))],
            &[source(1, 0, 10, 100), source(2, 0, 21, 200)]).unwrap();
        assert_eq!((added, cache.len()), (1, 1));

        let mut other = StrideCache::default();
        other.add_package(StrideSource::Enhanced, &[record(1, 0, 11, Some(1.0)), record(3, 0, 30, Some(2.0))],
            &[source(1, 0, 11, 100), source(3, 0, 30, 300)]).unwrap();
        cache.merge(other).unwrap();
        assert_eq!(cache.len(), 2);

        let mut conflicting = StrideCache::default();
        conflicting.add_package(StrideSource::Enhanced, &[record(1, 0, 10, Some(1.000_001))],
            &[source(1, 0, 10, 100)]).unwrap();
        let error = cache.merge(conflicting).unwrap_err().to_string();
        assert!(error.contains("ec body 1 action 0"), "{error}");
    }

    #[test]
    fn a_malformed_table_names_its_line() {
        let header = COLUMNS.join("\t");
        let row = "cc\t1\t0\t1.5\t-\t0.33333334\t1.5\t00000000000000ff";
        for (bad, line) in [
            (format!("# a comment\n{header}\tverdict\n"), 2),
            (format!("{header}\n{row}\n{row}\n"), 3),
            (format!("{header}\n{}\n", row.replace("cc", "kr")), 2),
            (format!("{header}\n{}\n", row.replace("\t1.5\t0000", "\t-1.5\t0000")), 2),
            (format!("{header}\n{}\n", row.replace("00ff", "00FF")), 2),
            (format!("{header}\n{}\n", row.replace("1.5\t-", "1.5\tx")), 2),
            (format!("{header}\n\n{}\n", row.replace("\t1.5\t0000", "\t0000")), 3),
        ] {
            let error = StrideCache::parse("bad", &bad).unwrap_err().to_string();
            assert!(error.starts_with(&format!("bad:{line}:")), "{error}");
        }
        assert_eq!(StrideCache::parse("good", &format!("{header}\n{row}\n")).unwrap().len(), 1);
    }

    /// The shipped table reads.
    #[test]
    fn the_shipped_table_reads() {
        let (path, _) = load_stride_cache(None).unwrap();
        assert_eq!(path.file_name().and_then(|name| name.to_str()), Some(STRIDE_CACHE_FILE_NAME));
    }
}
