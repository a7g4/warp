/// A vertical line in a plot, filling the full width
#[derive(Clone, Debug, PartialEq)]
pub struct ShadedXRange {
    base: egui_plot::PlotItemBase,
    x_min: f64,
    x_max: f64,
    fill: egui::Color32,
}

impl ShadedXRange {
    pub fn new(
        name: impl Into<String>,
        x_min: impl Into<f64>,
        x_max: impl Into<f64>,
        fill: impl Into<egui::Color32>,
    ) -> Self {
        Self {
            base: egui_plot::PlotItemBase::new(name.into()),
            x_min: x_min.into(),
            x_max: x_max.into(),
            fill: fill.into(),
        }
    }
}

impl egui_plot::PlotItem for ShadedXRange {
    fn shapes(&self, _ui: &egui::Ui, transform: &egui_plot::PlotTransform, shapes: &mut Vec<egui::Shape>) {
        // Get the current plot bounds to determine the full Y range
        let bounds = transform.bounds();
        let y_min = bounds.min()[1];
        let y_max = bounds.max()[1];

        // Create a polygon that spans the full Y range between x_min and x_max
        let points = vec![
            transform.position_from_point(&egui_plot::PlotPoint::new(self.x_min, y_min)),
            transform.position_from_point(&egui_plot::PlotPoint::new(self.x_max, y_min)),
            transform.position_from_point(&egui_plot::PlotPoint::new(self.x_max, y_max)),
            transform.position_from_point(&egui_plot::PlotPoint::new(self.x_min, y_max)),
        ];

        // Create a convex polygon shape
        shapes.push(egui::Shape::convex_polygon(points, self.fill, egui::Stroke::NONE));
    }

    fn initialize(&mut self, _x_range: std::ops::RangeInclusive<f64>) {
        // Copying from VLine implementation
    }

    fn color(&self) -> egui::Color32 {
        self.fill
    }

    fn geometry(&self) -> egui_plot::PlotGeometry<'_> {
        egui_plot::PlotGeometry::None
    }

    fn bounds(&self) -> egui_plot::PlotBounds {
        // Return bounds that only include the X range, not Y
        // This prevents the shaded area from affecting Y-axis scaling
        let mut min_bound = [f64::INFINITY; 2];
        min_bound[0] = self.x_min;
        min_bound[1] = f64::INFINITY; // Don't affect Y bounds

        let mut max_bound = [-f64::INFINITY; 2];
        max_bound[0] = self.x_max;
        max_bound[1] = -f64::INFINITY; // Don't affect Y bounds

        egui_plot::PlotBounds::from_min_max(min_bound, max_bound)
    }

    fn base(&self) -> &egui_plot::PlotItemBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut egui_plot::PlotItemBase {
        &mut self.base
    }
}
