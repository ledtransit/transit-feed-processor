use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result, bail};
use csv::ByteRecord;
use log::debug;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use zip::ZipArchive;

#[derive(Deserialize, Clone)]
pub struct AgencyRow {
    #[serde(default)]
    pub agency_id: String,
    pub agency_name: String,
    pub agency_timezone: String,
}

#[derive(Deserialize, Clone)]
pub struct RouteRow {
    pub route_id: String,
    #[serde(default)]
    pub agency_id: Option<String>,
    #[serde(default)]
    pub route_short_name: Option<String>,
    #[serde(default)]
    pub route_long_name: Option<String>,
    #[serde(default)]
    pub route_desc: Option<String>,
    pub route_type: i16,
    #[serde(default)]
    pub route_url: Option<String>,
    #[serde(default)]
    pub route_color: Option<String>,
    #[serde(default)]
    pub route_text_color: Option<String>,
    #[serde(default)]
    pub route_sort_order: Option<i32>,
    #[serde(default)]
    pub continuous_pickup: Option<i16>,
    #[serde(default)]
    pub continuous_drop_off: Option<i16>,
    #[serde(default)]
    pub network_id: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct TripRow {
    pub route_id: String,
    pub service_id: String,
    pub trip_id: String,
    #[serde(default)]
    pub trip_headsign: Option<String>,
    #[serde(default)]
    pub trip_short_name: Option<String>,
    #[serde(default)]
    pub direction_id: Option<i16>,
    #[serde(default)]
    pub block_id: Option<String>,
    #[serde(default)]
    pub shape_id: Option<String>,
    #[serde(default)]
    pub wheelchair_accessible: Option<i16>,
    #[serde(default)]
    pub bikes_allowed: Option<i16>,
}

#[derive(Deserialize, Clone)]
pub struct StopTimeRow {
    pub trip_id: String,
    #[serde(default)]
    pub arrival_time: Option<String>,
    #[serde(default)]
    pub departure_time: Option<String>,
    #[serde(default)]
    pub stop_id: String,
    pub stop_sequence: i32,
    #[serde(default)]
    pub stop_headsign: Option<String>,
    #[serde(default)]
    pub pickup_type: Option<i16>,
    #[serde(default)]
    pub drop_off_type: Option<i16>,
    #[serde(default)]
    pub continuous_pickup: Option<i16>,
    #[serde(default)]
    pub continuous_drop_off: Option<i16>,
    #[serde(default)]
    pub shape_dist_traveled: Option<f64>,
    #[serde(default)]
    pub timepoint: Option<i16>,
}

#[derive(Deserialize, Clone)]
pub struct StopRow {
    pub stop_id: String,
    #[serde(default)]
    pub stop_code: Option<String>,
    #[serde(default)]
    pub stop_name: Option<String>,
    #[serde(default)]
    pub tts_stop_name: Option<String>,
    #[serde(default)]
    pub stop_desc: Option<String>,
    #[serde(default)]
    pub stop_lat: Option<f64>,
    #[serde(default)]
    pub stop_lon: Option<f64>,
    #[serde(default)]
    pub zone_id: Option<String>,
    #[serde(default)]
    pub stop_url: Option<String>,
    #[serde(default)]
    pub location_type: Option<i16>,
    #[serde(default)]
    pub parent_station: Option<String>,
    #[serde(default)]
    pub stop_timezone: Option<String>,
    #[serde(default)]
    pub wheelchair_boarding: Option<i16>,
    #[serde(default)]
    pub level_id: Option<String>,
    #[serde(default)]
    pub platform_code: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct CalendarRow {
    pub service_id: String,
    pub monday: u8,
    pub tuesday: u8,
    pub wednesday: u8,
    pub thursday: u8,
    pub friday: u8,
    pub saturday: u8,
    pub sunday: u8,
    pub start_date: String,
    pub end_date: String,
}

#[derive(Deserialize, Clone)]
pub struct CalendarDateRow {
    pub service_id: String,
    pub date: String,
    pub exception_type: i16,
}

pub struct Tables {
    pub agencies: Vec<AgencyRow>,
    pub routes: Vec<RouteRow>,
    pub trips: Vec<TripRow>,
    pub stop_times: Vec<StopTimeRow>,
    pub stops: Vec<StopRow>,
    pub calendar: Vec<CalendarRow>,
    pub calendar_dates: Vec<CalendarDateRow>,
}

/// Where a feed's tables are read from, for [`super::filter::select_scopes`]:
/// the small tables whole, the others a row at a time. Each streamed table
/// takes `keep`, a test on the row's key column, and only rows passing it
/// are handed to `f` — so a streaming source never holds a whole table, nor
/// even parses the rows it leaves out.
pub trait RowSource {
    fn agencies(&mut self) -> Result<Vec<AgencyRow>>;
    fn routes(&mut self) -> Result<Vec<RouteRow>>;
    /// Keyed by `route_id`.
    fn trips(&mut self, keep: &dyn Fn(&str) -> bool, f: &mut dyn FnMut(TripRow)) -> Result<()>;
    /// Keyed by `trip_id`.
    fn stop_times(
        &mut self,
        keep: &dyn Fn(&str) -> bool,
        f: &mut dyn FnMut(StopTimeRow),
    ) -> Result<()>;
    /// Keyed by `stop_id`; may be read more than once.
    fn stops(&mut self, keep: &dyn Fn(&str) -> bool, f: &mut dyn FnMut(StopRow)) -> Result<()>;
    /// Keyed by `service_id`; absent tables have no rows.
    fn calendar(
        &mut self,
        keep: &dyn Fn(&str) -> bool,
        f: &mut dyn FnMut(CalendarRow),
    ) -> Result<()>;
    /// Keyed by `service_id`; absent tables have no rows.
    fn calendar_dates(
        &mut self,
        keep: &dyn Fn(&str) -> bool,
        f: &mut dyn FnMut(CalendarDateRow),
    ) -> Result<()>;
}

/// Tables already in memory: rows passing `keep` are handed out as copies.
impl RowSource for &Tables {
    fn agencies(&mut self) -> Result<Vec<AgencyRow>> {
        Ok(self.agencies.clone())
    }

    fn routes(&mut self) -> Result<Vec<RouteRow>> {
        Ok(self.routes.clone())
    }

    fn trips(&mut self, keep: &dyn Fn(&str) -> bool, f: &mut dyn FnMut(TripRow)) -> Result<()> {
        copy_where(&self.trips, |r| keep(&r.route_id), f)
    }

    fn stop_times(
        &mut self,
        keep: &dyn Fn(&str) -> bool,
        f: &mut dyn FnMut(StopTimeRow),
    ) -> Result<()> {
        copy_where(&self.stop_times, |r| keep(&r.trip_id), f)
    }

    fn stops(&mut self, keep: &dyn Fn(&str) -> bool, f: &mut dyn FnMut(StopRow)) -> Result<()> {
        copy_where(&self.stops, |r| keep(&r.stop_id), f)
    }

    fn calendar(
        &mut self,
        keep: &dyn Fn(&str) -> bool,
        f: &mut dyn FnMut(CalendarRow),
    ) -> Result<()> {
        copy_where(&self.calendar, |r| keep(&r.service_id), f)
    }

    fn calendar_dates(
        &mut self,
        keep: &dyn Fn(&str) -> bool,
        f: &mut dyn FnMut(CalendarDateRow),
    ) -> Result<()> {
        copy_where(&self.calendar_dates, |r| keep(&r.service_id), f)
    }
}

fn copy_where<T: Clone>(rows: &[T], keep: impl Fn(&T) -> bool, f: &mut dyn FnMut(T)) -> Result<()> {
    rows.iter().filter(|r| keep(r)).cloned().for_each(f);
    Ok(())
}

/// A key column's name, and the test its value must pass for the row to be
/// kept.
type KeyTest<'a> = (&'a str, &'a dyn Fn(&str) -> bool);

/// A downloaded GTFS zip, its text files read straight out of it.
pub struct Archive {
    zip: ZipArchive<File>,
    /// Each archived file's base name -> its entry index.
    index: HashMap<String, usize>,
}

impl Archive {
    pub fn open(zip_path: &Path) -> Result<Self> {
        let file = File::open(zip_path).with_context(|| format!("open {}", zip_path.display()))?;
        let mut zip = ZipArchive::new(file).context("read zip archive")?;
        let index = basename_index(&mut zip)?;
        Ok(Self { zip, index })
    }

    /// Every row of `name`, or none if it's absent and not `required`.
    fn read_all<T: DeserializeOwned>(&mut self, name: &str, required: bool) -> Result<Vec<T>> {
        let mut rows = Vec::new();
        self.stream(name, required, None, &mut |row| rows.push(row))?;
        Ok(rows)
    }

    /// Hand every row of `name` whose `key` column passes its test to `f`,
    /// parsing only those; none if the file's absent and not `required`.
    fn stream<T: DeserializeOwned>(
        &mut self,
        name: &str,
        required: bool,
        key: Option<KeyTest<'_>>,
        f: &mut dyn FnMut(T),
    ) -> Result<()> {
        let Some(&i) = self.index.get(name) else {
            if required {
                bail!("{name} missing from feed archive");
            }
            return Ok(());
        };
        let entry = self
            .zip
            .by_index(i)
            .with_context(|| format!("open {name}"))?;
        let reader = skip_bom(BufReader::with_capacity(1 << 16, entry))?;
        let mut csv = csv::ReaderBuilder::new().flexible(true).from_reader(reader);
        let headers = csv
            .byte_headers()
            .with_context(|| format!("parse {name}"))?
            .clone();
        let key = match key {
            Some((column, keep)) => {
                let at = headers
                    .iter()
                    .position(|h| h == column.as_bytes())
                    .with_context(|| format!("{name} has no {column} column"))?;
                Some((at, keep))
            }
            None => None,
        };

        let mut record = ByteRecord::new();
        let (mut read, mut kept) = (0usize, 0usize);
        while csv
            .read_byte_record(&mut record)
            .with_context(|| format!("parse {name}"))?
        {
            read += 1;
            if let Some((at, keep)) = key {
                let value = record
                    .get(at)
                    .and_then(|v| std::str::from_utf8(v).ok())
                    .with_context(|| format!("parse {name}: row {read} has no valid key"))?;
                if !keep(value) {
                    continue;
                }
            }
            let row = record
                .deserialize(Some(&headers))
                .with_context(|| format!("parse {name}"))?;
            kept += 1;
            f(row);
        }
        debug!("read {name}: {read} rows, kept {kept}");
        Ok(())
    }
}

impl RowSource for Archive {
    fn agencies(&mut self) -> Result<Vec<AgencyRow>> {
        self.read_all("agency.txt", true)
    }

    fn routes(&mut self) -> Result<Vec<RouteRow>> {
        self.read_all("routes.txt", true)
    }

    fn trips(&mut self, keep: &dyn Fn(&str) -> bool, f: &mut dyn FnMut(TripRow)) -> Result<()> {
        self.stream("trips.txt", true, Some(("route_id", keep)), f)
    }

    fn stop_times(
        &mut self,
        keep: &dyn Fn(&str) -> bool,
        f: &mut dyn FnMut(StopTimeRow),
    ) -> Result<()> {
        self.stream("stop_times.txt", true, Some(("trip_id", keep)), f)
    }

    fn stops(&mut self, keep: &dyn Fn(&str) -> bool, f: &mut dyn FnMut(StopRow)) -> Result<()> {
        self.stream("stops.txt", true, Some(("stop_id", keep)), f)
    }

    fn calendar(
        &mut self,
        keep: &dyn Fn(&str) -> bool,
        f: &mut dyn FnMut(CalendarRow),
    ) -> Result<()> {
        self.stream("calendar.txt", false, Some(("service_id", keep)), f)
    }

    fn calendar_dates(
        &mut self,
        keep: &dyn Fn(&str) -> bool,
        f: &mut dyn FnMut(CalendarDateRow),
    ) -> Result<()> {
        self.stream("calendar_dates.txt", false, Some(("service_id", keep)), f)
    }
}

/// Map each archived file's base name to its entry index
fn basename_index(zip: &mut ZipArchive<File>) -> Result<HashMap<String, usize>> {
    let mut index = HashMap::new();
    for i in 0..zip.len() {
        let entry = zip.by_index(i)?;
        if !entry.is_file() {
            continue;
        }
        if let Some(name) = entry
            .enclosed_name()
            .as_deref()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
        {
            index.insert(name, i);
        }
    }
    Ok(index)
}

/// UTF-8 byte order mark; some GTFS publishers prefix it to their CSV files,
/// which would otherwise glue onto the first header name.
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

fn skip_bom<R: BufRead>(mut r: R) -> std::io::Result<R> {
    if r.fill_buf()?.starts_with(&UTF8_BOM) {
        r.consume(UTF8_BOM.len());
    }
    Ok(r)
}
