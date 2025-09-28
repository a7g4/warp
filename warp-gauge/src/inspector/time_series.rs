#[derive(Debug, Clone)]
struct BinStats {
    x_center: f64,
    min: f64,
    max: f64,
    sum: f64,
    count: usize,
}

impl BinStats {
    fn new(x_center: f64, first_y: f64) -> Self {
        Self {
            x_center,
            min: first_y,
            max: first_y,
            sum: first_y,
            count: 1,
        }
    }

    fn add_point(&mut self, y: f64) {
        self.min = self.min.min(y);
        self.max = self.max.max(y);
        self.sum += y;
        self.count += 1;
    }

    fn mean(&self) -> f64 {
        self.sum / self.count as f64
    }
}

pub struct TimeSeries<'a> {
    base: egui_plot::PlotItemBase,
    color: egui::Color32,
    points: egui_plot::PlotPoints<'a>,
    bounds: egui_plot::PlotBounds,
    pixels_per_bin: u8,
}

impl<'a> TimeSeries<'a> {
    pub fn new(
        name: impl Into<String>,
        color: egui::Color32,
        pixels_per_bin: u8,
        points: egui_plot::PlotPoints<'a>,
    ) -> Self {
        let mut bounds = egui_plot::PlotBounds::NOTHING;
        points.points().iter().for_each(|p| bounds.extend_with(p));

        Self {
            base: egui_plot::PlotItemBase::new(name.into()),
            color,
            points,
            bounds,
            pixels_per_bin,
        }
    }
}

impl<'a> egui_plot::PlotItem for TimeSeries<'a> {
    fn shapes(&self, _: &egui::Ui, transform: &egui_plot::PlotTransform, shapes: &mut Vec<egui::Shape>) {
        let plot_space_per_ui_space = transform.dvalue_dpos();
        let plot_bounds = transform.bounds();

        let bin_width = plot_space_per_ui_space[0] * (self.pixels_per_bin as f64);

        let x_min = plot_bounds.min()[0];
        let x_max = plot_bounds.max()[0];

        let num_bins = ((x_max - x_min) / bin_width).ceil() as usize + 1;
        let mut bin_stats: Vec<Option<BinStats>> = vec![None; num_bins];

        // Single pass: accumulate all statistics
        for point in self.points.points() {
            if point.x >= x_min && point.x <= x_max {
                let bin_index = ((point.x - x_min) / bin_width).floor() as usize;
                let x_center = x_min + (bin_index as f64 + 0.5) * bin_width;

                match &mut bin_stats[bin_index] {
                    Some(stats) => {
                        stats.add_point(point.y);
                    }
                    None => {
                        bin_stats[bin_index] = Some(BinStats::new(x_center, point.y));
                    }
                }
            }
        }

        // Render min/max filled rectangles
        let [r, g, b, _] = self.color.to_array();
        let fill_color = egui::Color32::from_rgba_unmultiplied(r, g, b, 80);

        for bin_stat in bin_stats.iter().filter_map(|s| s.as_ref()) {
            let visual_width = self.pixels_per_bin as f32;
            let center_screen = transform.position_from_point(&egui_plot::PlotPoint::new(
                bin_stat.x_center,
                (bin_stat.min + bin_stat.max) / 2.0,
            ));
            let top_screen = transform.position_from_point(&egui_plot::PlotPoint::new(bin_stat.x_center, bin_stat.max));
            let bottom_screen =
                transform.position_from_point(&egui_plot::PlotPoint::new(bin_stat.x_center, bin_stat.min));

            let rect = egui::emath::Rect::from_two_pos(
                egui::Pos2::new(center_screen.x - visual_width / 2.0, top_screen.y.min(bottom_screen.y)),
                egui::Pos2::new(center_screen.x + visual_width / 2.0, top_screen.y.max(bottom_screen.y)),
            );

            if rect.width() > 0.0 && rect.height() > 0.0 {
                shapes.push(egui::epaint::Shape::rect_filled(rect, 0.0, fill_color));
            }
        }

        // Render mean line
        let mean_points: Vec<egui::Pos2> = bin_stats
            .iter()
            .filter_map(|bin_stat| {
                bin_stat.as_ref().map(|stats| {
                    transform.position_from_point(&egui_plot::PlotPoint::new(stats.x_center, stats.mean()))
                })
            })
            .collect();

        for window in mean_points.windows(2) {
            shapes.push(egui::epaint::Shape::line_segment(
                [window[0], window[1]],
                egui::epaint::Stroke::new(2.0, self.color),
            ));
        }
    }

    fn initialize(&mut self, _x_range: std::ops::RangeInclusive<f64>) {}

    fn color(&self) -> egui::Color32 {
        self.color
    }

    fn geometry(&self) -> egui_plot::PlotGeometry<'_> {
        egui_plot::PlotGeometry::None
    }

    fn bounds(&self) -> egui_plot::PlotBounds {
        self.bounds
    }

    fn base(&self) -> &egui_plot::PlotItemBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut egui_plot::PlotItemBase {
        &mut self.base
    }
}
