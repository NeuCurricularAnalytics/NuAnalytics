//! IPEDS data ingestion
//!
//! Parses locally downloaded IPEDS CSV files and ingests them into Supabase.
//!
//! ## Usage
//!
//! Download the following files from <https://nces.ed.gov/ipeds/use-the-data>:
//! - `HD{year}.zip` or `HD{year}.csv` — institution directory
//! - `C{year}_A.zip` or `C{year}_A.csv` — completions by award level
//! - `EF{year}A.zip` or `EF{year}A.csv` — fall enrollment
//!
//! Then run: `nuanalytics db ipeds-import --year 2023 --dir ./ipeds_data/`

pub mod ingest;

pub use ingest::{ingest_completions, ingest_institutions, is_relevant_cip, IngestStats};
