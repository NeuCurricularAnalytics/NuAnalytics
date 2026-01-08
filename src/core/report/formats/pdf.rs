//! PDF report generator via HTML-to-PDF conversion
//!
//! Generates PDF reports by first creating an HTML report and then converting
//! it to PDF using headless Chrome/Chromium or another specified converter.
//!
//! This approach provides:
//! - High-quality PDFs with proper graph rendering
//! - Same visualization as HTML reports (Mermaid diagrams)
//! - No dependency on complex PDF generation libraries

use super::html::HtmlReporter;
use crate::core::report::{ReportContext, ReportGenerator};
use std::error::Error;
use std::path::Path;
use std::process::Command;

/// PDF report generator using HTML-to-PDF conversion
pub struct PdfReporter {
    /// Optional custom PDF converter command
    converter: Option<String>,
}

impl PdfReporter {
    /// Create a new PDF reporter
    #[must_use]
    pub const fn new() -> Self {
        Self { converter: None }
    }

    /// Create a PDF reporter with a custom converter
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn with_converter(converter: &str) -> Self {
        Self {
            converter: Some(converter.to_owned()),
        }
    }

    /// Detect available Chrome/Chromium browser
    fn detect_chrome() -> Option<String> {
        // Try common Chrome/Chromium executables in order of preference
        let candidates = [
            "google-chrome",
            "chrome",
            "chromium",
            "chromium-browser",
            "google-chrome-stable",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", // macOS
            "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",   // Windows
            "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
        ];

        for candidate in candidates {
            if let Ok(output) = Command::new(candidate).arg("--version").output() {
                if output.status.success() {
                    return Some(candidate.to_owned());
                }
            }
        }

        None
    }

    /// Generate PDF from HTML file using Chrome/Chromium
    fn html_to_pdf_chrome(
        chrome_cmd: &str,
        html_path: &Path,
        pdf_path: &Path,
    ) -> Result<(), Box<dyn Error>> {
        // Suppress DBus warnings by redirecting stderr to /dev/null
        use std::process::Stdio;

        let status = Command::new(chrome_cmd)
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--no-sandbox")
            // Force complete rendering and JavaScript execution
            .arg("--run-all-compositor-stages-before-draw")
            // Extended timeout to ensure all JavaScript (including setTimeout) has time to complete
            .arg("--virtual-time-budget=60000") // 60 seconds for JS and timeouts to complete
            .arg("--disable-features=IsolateOrigins,site-per-process")
            .arg("--enable-features=NetworkService,NetworkServiceInProcess")
            // Force synchronous painting and wait for layout
            .arg("--enable-automation")
            // Ensure lazy loading doesn't interfere
            .arg("--disable-lazy-loading")
            .arg(format!("--print-to-pdf={}", pdf_path.display()))
            .arg(format!("file://{}", html_path.canonicalize()?.display()))
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .status()?;

        if !status.success() {
            return Err("Chrome PDF conversion failed".into());
        }

        Ok(())
    }

    /// Convert HTML report to PDF
    fn convert_html_to_pdf(&self, html_path: &Path, pdf_path: &Path) -> Result<(), Box<dyn Error>> {
        // Use custom converter if provided
        if let Some(converter) = &self.converter {
            return Self::html_to_pdf_chrome(converter, html_path, pdf_path);
        }

        // Try to auto-detect Chrome/Chromium
        if let Some(chrome) = Self::detect_chrome() {
            return Self::html_to_pdf_chrome(&chrome, html_path, pdf_path);
        }

        // No converter available
        Err("PDF conversion failed: Chrome/Chromium not found.\n\
            \n\
            To generate PDF reports, install Chrome or Chromium:\n\
            \n\
            • Ubuntu/Debian:  sudo apt install chromium-browser\n\
            • Fedora/RHEL:    sudo dnf install chromium\n\
            • macOS:          brew install --cask google-chrome\n\
            • Windows:        Download from https://www.google.com/chrome/\n\
            \n\
            Alternatively, specify a custom PDF converter:\n\
              --pdf-converter /path/to/chrome\n\
            "
        .into())
    }
}

impl Default for PdfReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ReportGenerator for PdfReporter {
    /// Generate PDF report via HTML-to-PDF conversion
    ///
    /// First generates an HTML report, then converts it to PDF using
    /// headless Chrome/Chromium or a specified converter.
    fn generate(&self, ctx: &ReportContext, output_path: &Path) -> Result<(), Box<dyn Error>> {
        // Generate HTML report to temporary file
        let temp_dir = std::env::temp_dir();
        let html_path = temp_dir.join(format!("nuanalytics_report_{}.html", std::process::id()));

        let html_reporter = HtmlReporter::new();
        html_reporter.generate(ctx, &html_path)?;

        // Convert HTML to PDF
        self.convert_html_to_pdf(&html_path, output_path)?;

        // Clean up temporary HTML file
        let _ = std::fs::remove_file(&html_path);

        Ok(())
    }

    /// Render method for consistency with other reporters
    fn render(&self, _ctx: &ReportContext) -> Result<String, Box<dyn Error>> {
        Ok(String::from(
            "PDF reports are generated via HTML-to-PDF conversion.",
        ))
    }
}
