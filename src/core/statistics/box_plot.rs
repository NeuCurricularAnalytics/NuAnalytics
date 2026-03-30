//! Box plot SVG generation for degree analysis
//!
//! Generates SVG box plots for visualizing metric distributions across
//! multiple degree plans. Uses pure Rust SVG generation without external
//! dependencies.

// Allow format string allocations for SVG generation - clearer than write! macros
#![allow(clippy::format_push_string)]

use super::aggregator::MetricStats;

/// Configuration for box plot rendering
#[derive(Debug, Clone)]
pub struct BoxPlotConfig {
    /// Width of the SVG in pixels
    pub width: f64,
    /// Height of the SVG in pixels
    pub height: f64,
    /// Left margin for labels
    pub margin_left: f64,
    /// Right margin
    pub margin_right: f64,
    /// Top margin
    pub margin_top: f64,
    /// Bottom margin for axis labels
    pub margin_bottom: f64,
    /// Box width as fraction of available space
    pub box_width_ratio: f64,
    /// Primary color for box fill
    pub box_fill: String,
    /// Color for box stroke
    pub box_stroke: String,
    /// Color for median line
    pub median_color: String,
    /// Color for whiskers
    pub whisker_color: String,
    /// Font size for labels
    pub font_size: f64,
    /// Font family
    pub font_family: String,
}

impl Default for BoxPlotConfig {
    fn default() -> Self {
        Self {
            width: 400.0,
            height: 200.0,
            margin_left: 60.0,
            margin_right: 30.0,
            margin_top: 30.0,
            margin_bottom: 40.0,
            box_width_ratio: 0.5,
            box_fill: "#e3f2fd".to_string(),
            box_stroke: "#1976d2".to_string(),
            median_color: "#d32f2f".to_string(),
            whisker_color: "#424242".to_string(),
            font_size: 12.0,
            font_family: "sans-serif".to_string(),
        }
    }
}

/// Data for a single box plot
#[derive(Debug, Clone)]
pub struct BoxPlotData {
    /// Label for this box plot
    pub label: String,
    /// Minimum value
    pub min: f64,
    /// First quartile (Q1)
    pub q1: f64,
    /// Median (Q2)
    pub median: f64,
    /// Third quartile (Q3)
    pub q3: f64,
    /// Maximum value
    pub max: f64,
    /// Optional mean value (shown as diamond)
    pub mean: Option<f64>,
}

impl BoxPlotData {
    /// Create box plot data from metric statistics
    #[must_use]
    pub fn from_metric_stats(label: &str, stats: &MetricStats) -> Self {
        Self {
            label: label.to_string(),
            min: stats.min,
            q1: stats.q1,
            median: stats.median,
            q3: stats.q3,
            max: stats.max,
            mean: Some(stats.mean),
        }
    }

    /// Create box plot data from raw values
    #[must_use]
    pub fn new(label: &str, min: f64, q1: f64, median: f64, q3: f64, max: f64) -> Self {
        Self {
            label: label.to_string(),
            min,
            q1,
            median,
            q3,
            max,
            mean: None,
        }
    }

    /// Set the mean value
    #[must_use]
    pub const fn with_mean(mut self, mean: f64) -> Self {
        self.mean = Some(mean);
        self
    }

    /// Get the range (max - min)
    #[must_use]
    pub fn range(&self) -> f64 {
        self.max - self.min
    }
}

/// Box plot SVG generator
pub struct BoxPlotGenerator {
    /// Configuration
    config: BoxPlotConfig,
}

impl BoxPlotGenerator {
    /// Create a new box plot generator with default config
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: BoxPlotConfig::default(),
        }
    }

    /// Create with custom configuration
    #[must_use]
    pub const fn with_config(config: BoxPlotConfig) -> Self {
        Self { config }
    }

    /// Generate SVG for a single box plot with title
    ///
    /// # Arguments
    /// * `title` - Title displayed above the plot
    /// * `data` - Box plot data
    ///
    /// # Returns
    /// SVG string
    #[must_use]
    pub fn generate_single(&self, title: &str, data: &BoxPlotData) -> String {
        self.generate(title, std::slice::from_ref(data))
    }

    /// Generate SVG for multiple box plots (side by side)
    ///
    /// # Arguments
    /// * `title` - Title displayed above the plots
    /// * `data` - Vector of box plot data
    ///
    /// # Returns
    /// SVG string
    #[must_use]
    pub fn generate(&self, title: &str, data: &[BoxPlotData]) -> String {
        if data.is_empty() {
            return self.generate_empty_plot(title);
        }

        let cfg = &self.config;
        let plot_width = cfg.width - cfg.margin_left - cfg.margin_right;
        let plot_height = cfg.height - cfg.margin_top - cfg.margin_bottom;

        // Find global min/max for scaling
        let (global_min, global_max) = Self::compute_scale_range(data);
        let value_range = global_max - global_min;

        // Avoid division by zero
        let value_range = if value_range < f64::EPSILON {
            1.0
        } else {
            value_range
        };

        let mut svg = String::new();

        // SVG header
        svg.push_str(&format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {:.0} {:.0}" width="{:.0}" height="{:.0}">"#,
            cfg.width, cfg.height, cfg.width, cfg.height
        ));

        // Background
        svg.push_str(&format!(
            r#"<rect width="{:.0}" height="{:.0}" fill="white"/>"#,
            cfg.width, cfg.height
        ));

        // Title
        svg.push_str(&format!(
            r#"<text x="{:.1}" y="{:.1}" font-family="{}" font-size="{:.0}" font-weight="bold" text-anchor="middle">{}</text>"#,
            cfg.width / 2.0,
            cfg.margin_top / 2.0 + 5.0,
            cfg.font_family,
            cfg.font_size + 2.0,
            Self::escape_xml(title)
        ));

        // Y-axis
        svg.push_str(&self.generate_y_axis(global_min, global_max, plot_height));

        // Generate each box plot
        #[allow(clippy::cast_precision_loss)]
        let box_spacing = plot_width / data.len() as f64;
        let box_width = box_spacing * cfg.box_width_ratio;

        for (i, box_data) in data.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let center_x = (i as f64 + 0.5).mul_add(box_spacing, cfg.margin_left);
            svg.push_str(&self.generate_box(
                box_data,
                center_x,
                box_width,
                plot_height,
                global_min,
                value_range,
            ));
        }

        svg.push_str("</svg>");
        svg
    }

    /// Generate Y-axis with tick marks and labels
    fn generate_y_axis(&self, min: f64, max: f64, plot_height: f64) -> String {
        let cfg = &self.config;
        let mut svg = String::new();

        // Axis line
        svg.push_str(&format!(
            r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" stroke-width="1"/>"#,
            cfg.margin_left,
            cfg.margin_top,
            cfg.margin_left,
            cfg.margin_top + plot_height,
            cfg.whisker_color
        ));

        // Generate ~5 tick marks
        let tick_count = 5;
        let value_range = max - min;
        let tick_step = value_range / f64::from(tick_count);

        for i in 0..=tick_count {
            let value = f64::from(i).mul_add(tick_step, min);
            let normalized = (value - min) / value_range;
            let y = normalized.mul_add(-plot_height, cfg.margin_top + plot_height);

            // Tick mark
            svg.push_str(&format!(
                r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" stroke-width="1"/>"#,
                cfg.margin_left - 5.0,
                y,
                cfg.margin_left,
                y,
                cfg.whisker_color
            ));

            // Label
            svg.push_str(&format!(
                r#"<text x="{:.1}" y="{:.1}" font-family="{}" font-size="{:.0}" text-anchor="end" dominant-baseline="middle">{:.1}</text>"#,
                cfg.margin_left - 8.0,
                y,
                cfg.font_family,
                cfg.font_size - 2.0,
                value
            ));

            // Grid line (light)
            if i > 0 && i < tick_count {
                svg.push_str(&format!(
                    "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"#e0e0e0\" stroke-width=\"1\" stroke-dasharray=\"3,3\"/>",
                    cfg.margin_left,
                    y,
                    cfg.width - cfg.margin_right,
                    y
                ));
            }
        }

        svg
    }

    /// Generate a single box plot element
    #[allow(clippy::too_many_arguments)]
    fn generate_box(
        &self,
        data: &BoxPlotData,
        center_x: f64,
        box_width: f64,
        plot_height: f64,
        min_value: f64,
        value_range: f64,
    ) -> String {
        let cfg = &self.config;
        let mut svg = String::new();

        // Helper to convert value to Y coordinate
        let value_to_y = |v: f64| -> f64 {
            let normalized = (v - min_value) / value_range;
            normalized.mul_add(-plot_height, cfg.margin_top + plot_height)
        };

        let y_min = value_to_y(data.min);
        let y_q1 = value_to_y(data.q1);
        let y_median = value_to_y(data.median);
        let y_q3 = value_to_y(data.q3);
        let y_max = value_to_y(data.max);

        let half_width = box_width / 2.0;
        let whisker_width = box_width / 3.0;

        // Lower whisker (min to Q1)
        svg.push_str(&format!(
            r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" stroke-width="1.5"/>"#,
            center_x, y_min, center_x, y_q1, cfg.whisker_color
        ));

        // Lower whisker cap
        svg.push_str(&format!(
            r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" stroke-width="1.5"/>"#,
            center_x - whisker_width,
            y_min,
            center_x + whisker_width,
            y_min,
            cfg.whisker_color
        ));

        // Upper whisker (Q3 to max)
        svg.push_str(&format!(
            r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" stroke-width="1.5"/>"#,
            center_x, y_q3, center_x, y_max, cfg.whisker_color
        ));

        // Upper whisker cap
        svg.push_str(&format!(
            r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" stroke-width="1.5"/>"#,
            center_x - whisker_width,
            y_max,
            center_x + whisker_width,
            y_max,
            cfg.whisker_color
        ));

        // Box (Q1 to Q3)
        let box_height = (y_q1 - y_q3).abs();
        svg.push_str(&format!(
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{}" stroke="{}" stroke-width="2"/>"#,
            center_x - half_width,
            y_q3,
            box_width,
            box_height,
            cfg.box_fill,
            cfg.box_stroke
        ));

        // Median line
        svg.push_str(&format!(
            r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" stroke-width="2.5"/>"#,
            center_x - half_width,
            y_median,
            center_x + half_width,
            y_median,
            cfg.median_color
        ));

        // Mean marker (diamond)
        if let Some(mean) = data.mean {
            let y_mean = value_to_y(mean);
            let diamond_size = 4.0;
            svg.push_str(&format!(
                "<polygon points=\"{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} {:.1},{:.1}\" fill=\"#4caf50\" stroke=\"#388e3c\" stroke-width=\"1\"/>",
                center_x,
                y_mean - diamond_size,
                center_x + diamond_size,
                y_mean,
                center_x,
                y_mean + diamond_size,
                center_x - diamond_size,
                y_mean
            ));
        }

        // X-axis label
        svg.push_str(&format!(
            r#"<text x="{:.1}" y="{:.1}" font-family="{}" font-size="{:.0}" text-anchor="middle">{}</text>"#,
            center_x,
            cfg.height - cfg.margin_bottom / 3.0,
            cfg.font_family,
            cfg.font_size,
            Self::escape_xml(&data.label)
        ));

        svg
    }

    /// Generate an empty plot with message
    fn generate_empty_plot(&self, title: &str) -> String {
        let cfg = &self.config;
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {:.0} {:.0}\" width=\"{:.0}\" height=\"{:.0}\">\n\
<rect width=\"{:.0}\" height=\"{:.0}\" fill=\"white\"/>\n\
<text x=\"{:.1}\" y=\"{:.1}\" font-family=\"{}\" font-size=\"{:.0}\" font-weight=\"bold\" text-anchor=\"middle\">{}</text>\n\
<text x=\"{:.1}\" y=\"{:.1}\" font-family=\"{}\" font-size=\"{:.0}\" text-anchor=\"middle\" fill=\"#666\">No data available</text>\n\
</svg>",
            cfg.width,
            cfg.height,
            cfg.width,
            cfg.height,
            cfg.width,
            cfg.height,
            cfg.width / 2.0,
            cfg.margin_top / 2.0 + 5.0,
            cfg.font_family,
            cfg.font_size + 2.0,
            Self::escape_xml(title),
            cfg.width / 2.0,
            cfg.height / 2.0,
            cfg.font_family,
            cfg.font_size
        )
    }

    /// Compute the min/max range for scaling, with 10% padding
    fn compute_scale_range(data: &[BoxPlotData]) -> (f64, f64) {
        let global_min = data.iter().map(|d| d.min).fold(f64::INFINITY, f64::min);
        let global_max = data.iter().map(|d| d.max).fold(f64::NEG_INFINITY, f64::max);

        let range = global_max - global_min;
        let padding = range * 0.1;

        // Ensure minimum is non-negative for most metrics
        let padded_min = (global_min - padding).max(0.0);
        let padded_max = global_max + padding;

        (padded_min, padded_max)
    }

    /// Escape XML special characters
    fn escape_xml(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }
}

impl Default for BoxPlotGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate a comparison box plot with multiple metrics
///
/// Creates a wider SVG with multiple box plots side by side.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn generate_comparison_plot(title: &str, data: &[BoxPlotData]) -> String {
    let width = 120.0f64.mul_add(data.len().max(2) as f64, 90.0);
    let config = BoxPlotConfig {
        width,
        height: 250.0,
        ..Default::default()
    };
    BoxPlotGenerator::with_config(config).generate(title, data)
}

/// Generate a simple single-metric box plot
#[must_use]
pub fn generate_single_plot(title: &str, stats: &MetricStats) -> String {
    let data = BoxPlotData::from_metric_stats(title, stats);
    BoxPlotGenerator::new().generate_single(title, &data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_stats() -> MetricStats {
        MetricStats {
            min: 50.0,
            max: 200.0,
            mean: 125.0,
            std_dev: 30.0,
            median: 120.0,
            q1: 90.0,
            q3: 160.0,
        }
    }

    #[test]
    fn test_box_plot_data_from_stats() {
        let stats = sample_stats();
        let data = BoxPlotData::from_metric_stats("Test", &stats);

        assert_eq!(data.label, "Test");
        assert!((data.min - 50.0).abs() < f64::EPSILON);
        assert!((data.max - 200.0).abs() < f64::EPSILON);
        assert!((data.median - 120.0).abs() < f64::EPSILON);
        assert!(data.mean.is_some());
    }

    #[test]
    fn test_box_plot_data_new() {
        let data = BoxPlotData::new("Test", 10.0, 25.0, 50.0, 75.0, 100.0);

        assert_eq!(data.label, "Test");
        assert!((data.min - 10.0).abs() < f64::EPSILON);
        assert!((data.max - 100.0).abs() < f64::EPSILON);
        assert!(data.mean.is_none());
    }

    #[test]
    fn test_box_plot_data_with_mean() {
        let data = BoxPlotData::new("Test", 10.0, 25.0, 50.0, 75.0, 100.0).with_mean(55.0);

        assert!(data.mean.is_some());
        assert!((data.mean.unwrap() - 55.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_box_plot_data_range() {
        let data = BoxPlotData::new("Test", 10.0, 25.0, 50.0, 75.0, 100.0);
        assert!((data.range() - 90.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_generator_default() {
        let gen = BoxPlotGenerator::default();
        assert!((gen.config.width - 400.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_generate_single_produces_svg() {
        let gen = BoxPlotGenerator::new();
        let data = BoxPlotData::new("Complexity", 50.0, 75.0, 100.0, 125.0, 150.0);
        let svg = gen.generate_single("Degree Complexity", &data);

        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("Degree Complexity"));
        assert!(svg.contains("Complexity")); // x-axis label
    }

    #[test]
    fn test_generate_multiple_produces_svg() {
        let gen = BoxPlotGenerator::new();
        let data = vec![
            BoxPlotData::new("A", 10.0, 20.0, 30.0, 40.0, 50.0),
            BoxPlotData::new("B", 15.0, 25.0, 35.0, 45.0, 55.0),
        ];
        let svg = gen.generate("Multiple Metrics", &data);

        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("Multiple Metrics"));
        assert!(svg.contains(">A</text>"));
        assert!(svg.contains(">B</text>"));
    }

    #[test]
    fn test_generate_empty_produces_svg() {
        let gen = BoxPlotGenerator::new();
        let svg = gen.generate("Empty Plot", &[]);

        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("No data available"));
    }

    #[test]
    fn test_generate_with_mean_marker() {
        let gen = BoxPlotGenerator::new();
        let data = BoxPlotData::new("Test", 10.0, 25.0, 50.0, 75.0, 100.0).with_mean(55.0);
        let svg = gen.generate_single("With Mean", &data);

        assert!(svg.contains("polygon")); // Diamond marker
        assert!(svg.contains("#4caf50")); // Mean marker fill color
    }

    #[test]
    fn test_escape_xml() {
        assert_eq!(
            BoxPlotGenerator::escape_xml("Test & <Data>"),
            "Test &amp; &lt;Data&gt;"
        );
    }

    #[test]
    fn test_comparison_plot() {
        let data = vec![
            BoxPlotData::new("Complexity", 50.0, 75.0, 100.0, 125.0, 150.0),
            BoxPlotData::new("Delay", 1.0, 3.0, 5.0, 7.0, 10.0),
        ];
        let svg = generate_comparison_plot("Degree Metrics", &data);

        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("Degree Metrics"));
    }

    #[test]
    fn test_single_plot_helper() {
        let stats = sample_stats();
        let svg = generate_single_plot("Complexity", &stats);

        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("Complexity"));
    }

    #[test]
    fn test_config_customization() {
        let config = BoxPlotConfig {
            width: 600.0,
            height: 300.0,
            box_fill: "#ff0000".to_string(),
            ..Default::default()
        };
        let gen = BoxPlotGenerator::with_config(config);
        let data = BoxPlotData::new("Test", 10.0, 25.0, 50.0, 75.0, 100.0);
        let svg = gen.generate_single("Custom", &data);

        assert!(svg.contains("600"));
        assert!(svg.contains("#ff0000"));
    }
}
