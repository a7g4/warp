pub(crate) mod histogram;
pub(crate) mod shaded_range;
pub(crate) mod time_series;

fn load_csv_data(file_path: &str) -> Result<DataSet, anyhow::Error> {
    let file = std::fs::File::open(file_path)?;
    let mut reader = csv::ReaderBuilder::new().has_headers(true).from_reader(file);

    let mut points = Vec::new();

    for result in reader.deserialize() {
        let point: crate::DataPoint = result?;
        points.push(point);
    }

    Ok(DataSet { points })
}

fn percentile(sorted_data: &[f64], p: f64) -> f64 {
    if sorted_data.is_empty() {
        return 0.0;
    }
    let index = (p * (sorted_data.len() - 1) as f64) as usize;
    sorted_data[index.min(sorted_data.len() - 1)]
}

fn calculate_statistics(points: &[crate::DataPoint]) -> DataStatistics {
    if points.is_empty() {
        return DataStatistics {
            min_latency: 0.0,
            max_latency: 0.0,
            mean_latency: 0.0,
            p50_latency: 0.0,
            p90_latency: 0.0,
            p99_latency: 0.0,
            packet_drop_percentage: 0.0,
            out_of_order_percentage: 0.0,
            data_point_count: 0,
        };
    }

    let mut latencies: Vec<f64> = points.iter().map(|p| p.latency_ms).collect();
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let min_latency = latencies[0];
    let max_latency = latencies[latencies.len() - 1];
    let mean_latency = latencies.iter().sum::<f64>() / latencies.len() as f64;

    let p50_latency = percentile(&latencies, 0.5);
    let p90_latency = percentile(&latencies, 0.9);
    let p99_latency = percentile(&latencies, 0.99);

    let (packet_drop_percentage, out_of_order_percentage) = calculate_packet_metrics(points);

    DataStatistics {
        min_latency,
        max_latency,
        mean_latency,
        p50_latency,
        p90_latency,
        p99_latency,
        packet_drop_percentage,
        out_of_order_percentage,
        data_point_count: points.len(),
    }
}

fn calculate_packet_metrics(points: &[crate::DataPoint]) -> (f64, f64) {
    if points.len() < 2 {
        return (0.0, 0.0);
    }

    // Extract min/max counter values.
    let min_counter = points.iter().min_by_key(|p| p.counter).unwrap().counter;
    let max_counter = points.iter().max_by_key(|p| p.counter).unwrap().counter;

    // Count out-of-order pairs (where the later point has a smaller counter).
    let out_of_order = points
        .windows(2)
        .filter(|pair| pair[1].counter < pair[0].counter)
        .count();

    // Compute percentages.
    let expected_packets = (max_counter - min_counter + 1) as f64;
    let packet_drop_percentage = if expected_packets > 0.0 {
        100.0 * (expected_packets - points.len() as f64) / expected_packets
    } else {
        0.0
    };

    let out_of_order_percentage = 100.0 * out_of_order as f64 / (points.len() - 1) as f64;

    (packet_drop_percentage, out_of_order_percentage)
}

#[derive(Debug, Clone)]
struct DataStatistics {
    min_latency: f64,
    max_latency: f64,
    mean_latency: f64,
    p50_latency: f64,
    p90_latency: f64,
    p99_latency: f64,
    packet_drop_percentage: f64,
    out_of_order_percentage: f64,
    data_point_count: usize,
}

#[derive(Debug, Clone)]
struct DataSet {
    points: Vec<crate::DataPoint>,
}
#[derive(Default)]
pub struct Inspector {
    data_set: Option<DataSet>,
    selected_x_range: Option<(f64, f64)>, // Store selected x-axis range (min, max)
    selection_start: Option<f64>,         // Start x-coordinate of selection
    is_selecting: bool,                   // Whether we're currently in selection mode
    load_error: Option<String>,           // Error message if loading failed
                                          //stats_expanded: bool,                 // Track if statistics are expanded
}

impl Inspector {
    fn load_data(&mut self) {
        // Open file dialog to select CSV file
        if let Some(file_path) = rfd::FileDialog::new()
            .add_filter("CSV files", &["csv"])
            .add_filter("All files", &["*"])
            .pick_file()
        {
            self.load_error = None;

            match load_csv_data(file_path.to_str().unwrap_or("")) {
                Ok(data_set) => {
                    self.data_set = Some(data_set);
                }
                Err(e) => {
                    self.load_error = Some(format!("Failed to load CSV: {e}"));
                }
            }
        }
    }

    fn get_selected_data(&self) -> Option<Vec<&crate::DataPoint>> {
        if let Some(ref data_set) = self.data_set
            && let Some((min_x, max_x)) = self.selected_x_range
        {
            let selected_points: Vec<&crate::DataPoint> = data_set
                .points
                .iter()
                .filter(|point| {
                    let counter = point.counter as f64;
                    counter >= min_x && counter <= max_x
                })
                .collect();

            if !selected_points.is_empty() {
                return Some(selected_points);
            }
        }
        None
    }

    fn generate_latency_data(&self) -> Vec<[f64; 2]> {
        if let Some(ref data_set) = self.data_set {
            data_set
                .points
                .iter()
                .map(|p| [p.counter as f64, p.latency_ms])
                .collect()
        } else {
            vec![]
        }
    }

    fn generate_histogram_data(&self) -> egui_plot::BarChart {
        if let Some(selected_data) = self.get_selected_data() {
            let latencies: Vec<f64> = selected_data.iter().map(|p| p.latency_ms).collect();
            let (histogram, bin_width) = crate::inspector::histogram::calculate_histogram(&latencies);

            // Create bar chart data
            let bars: Vec<egui_plot::Bar> = histogram
                .into_iter()
                .map(|(bin_center, percentage)| {
                    egui_plot::Bar::new(bin_center, percentage)
                        .stroke(egui::Stroke::NONE)
                        .width(bin_width) // Width based on actual bin width
                        .name(format!("{bin_center:.6} ms ({percentage:.1}%)"))
                })
                .collect();

            egui_plot::BarChart::new("latency_histogram", bars)
                .name("Latency Histogram")
                .color(egui::Color32::from_rgb(100, 150, 250))
        } else {
            egui_plot::BarChart::new("latency_histogram", vec![])
        }
    }

    fn generate_latency_vs_receiver_pps_data(&self) -> Vec<[f64; 2]> {
        if let Some(selected_data) = self.get_selected_data() {
            selected_data
                .iter()
                .map(|p| [p.receiver_calculated_pps as f64, p.latency_ms])
                .collect()
        } else {
            vec![]
        }
    }

    fn get_statistics(&self) -> Option<DataStatistics> {
        if let Some(selected_data) = self.get_selected_data() {
            let points: Vec<crate::DataPoint> = selected_data.iter().map(|p| (*p).clone()).collect();
            let stats = calculate_statistics(&points);
            Some(stats)
        } else {
            None
        }
    }

    // Plot PPS v/s counter
    fn render_pps_plot(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> egui_plot::PlotResponse<()> {
        let available_size = ui.available_size();

        // Check for Shift key to determine selection mode
        let shift_pressed = ui.input(|i| i.modifiers.shift);

        let legend = egui_plot::Legend::default();

        let data_set = &self.data_set.as_ref();

        let response = egui_plot::Plot::new("PPS Plot")
            .width(available_size.x)
            .height(available_size.y)
            .link_axis("left_plots_x", [true, false])
            .allow_drag(!shift_pressed)
            .allow_zoom(true)
            .allow_boxed_zoom(false)
            .legend(legend)
            .show(ui, |plot_ui| {
                if let Some(data_set) = data_set {
                    // Target PPS using TimeSeries
                    let target_pps_data: Vec<[f64; 2]> = data_set
                        .points
                        .iter()
                        .map(|p| [p.counter as f64, p.target_pps as f64])
                        .collect();

                    plot_ui.add(time_series::TimeSeries::new(
                        "Target PPS",
                        egui::Color32::from_rgb(100, 150, 250),
                        1,
                        target_pps_data.into(),
                    ));

                    // Sender PPS using TimeSeries (measured data with variance)
                    let sender_pps_data: Vec<[f64; 2]> = data_set
                        .points
                        .iter()
                        .map(|p| [p.counter as f64, p.sender_achieved_pps as f64])
                        .collect();

                    plot_ui.add(time_series::TimeSeries::new(
                        "Sender PPS",
                        egui::Color32::from_rgb(250, 150, 100),
                        1,
                        sender_pps_data.into(),
                    ));

                    // Receiver PPS using TimeSeries (measured data with variance)
                    let receiver_pps_data: Vec<[f64; 2]> = data_set
                        .points
                        .iter()
                        .map(|p| [p.counter as f64, p.receiver_calculated_pps as f64])
                        .collect();

                    plot_ui.add(time_series::TimeSeries::new(
                        "Receiver PPS",
                        egui::Color32::from_rgb(150, 250, 100),
                        1,
                        receiver_pps_data.into(),
                    ));
                }

                if let Some((min_x, max_x)) = self.selected_x_range {
                    let shaded_x_range = crate::inspector::shaded_range::ShadedXRange::new(
                        "", // Empty name hides it in the legend
                        min_x,
                        max_x,
                        egui::Color32::from_rgba_unmultiplied(100, 150, 250, 40),
                    );
                    plot_ui.add(shaded_x_range);
                }
            });

        // Handle selection
        self.handle_plot_selection(ui, ctx, &response, shift_pressed);

        response
    }

    // Helper method to render Latency plot
    fn render_latency_plot(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let available_size = ui.available_size();

        // Check for Shift key to determine selection mode
        let shift_pressed = ui.input(|i| i.modifiers.shift);

        let response = egui_plot::Plot::new("Latency Plot")
            .width(available_size.x)
            .height(available_size.y)
            .link_axis("left_plots_x", [true, false])
            .allow_drag(!shift_pressed)
            .allow_zoom(true)
            .allow_boxed_zoom(false)
            .show(ui, |plot_ui| {
                let latency_points = self.generate_latency_data();
                if !latency_points.is_empty() {
                    plot_ui.add(time_series::TimeSeries::new(
                        "asdf",
                        egui::Color32::RED,
                        1,
                        latency_points.into(),
                    ));
                }

                if let Some((min_x, max_x)) = self.selected_x_range {
                    let shaded_x_range = crate::inspector::shaded_range::ShadedXRange::new(
                        "", // Empty name hides it in the legend
                        min_x,
                        max_x,
                        egui::Color32::from_rgba_unmultiplied(100, 150, 250, 40),
                    );
                    plot_ui.add(shaded_x_range);
                }
            });

        // Handle selection
        self.handle_plot_selection(ui, ctx, &response, shift_pressed);
    }

    // Helper method to render Histogram plot
    fn render_histogram_plot(&mut self, ui: &mut egui::Ui) {
        let available_size = ui.available_size();

        egui_plot::Plot::new("histogram_plot")
            .width(available_size.x)
            .height(available_size.y)
            .show(ui, |plot_ui| {
                let histogram_chart = self.generate_histogram_data();
                plot_ui.bar_chart(histogram_chart);
            });
    }

    // Helper method to render Scatter plot
    fn render_scatter_plot(&mut self, ui: &mut egui::Ui) {
        let available_size = ui.available_size();

        egui_plot::Plot::new("Latency v/s PPS")
            .width(available_size.x)
            .height(available_size.y)
            .y_axis_min_width(10.0)
            .show(ui, |plot_ui| {
                let scatter_data = self.generate_latency_vs_receiver_pps_data();
                if !scatter_data.is_empty() {
                    let scatter_points = egui_plot::Points::new("latency_vs_receiver", scatter_data)
                        .color(egui::Color32::from_rgb(250, 100, 150))
                        .name("Latency vs Receiver PPS");
                    plot_ui.points(scatter_points);
                }
            });
    }

    // Helper method to render collapsible Statistics section
    fn render_collapsible_statistics(&mut self, ui: &mut egui::Ui) -> egui::CollapsingResponse<()> {
        // Track the expansion state
        egui::CollapsingHeader::new("Statistics")
            .default_open(false)
            .show(ui, |ui| {
                if let Some(stats) = self.get_statistics() {
                    ui.add_space(5.0);

                    // Use columns for better space utilization
                    ui.columns(3, |columns| {
                        // Column 1: Min, Mean, Max
                        columns[0].vertical(|ui| {
                            ui.label(format!("Min: {:.6} ms", stats.min_latency * 1e3));
                            ui.label(format!("Mean: {:.6} ms", stats.mean_latency * 1e3));
                            ui.label(format!("Max: {:.6} ms", stats.max_latency * 1e3));
                        });

                        // Column 2: P50, P90, P99
                        columns[1].vertical(|ui| {
                            ui.label(format!("P50: {:.6} ms", stats.p50_latency * 1e3));
                            ui.label(format!("P90: {:.6} ms", stats.p90_latency * 1e3));
                            ui.label(format!("P99: {:.6} ms", stats.p99_latency * 1e3));
                        });

                        // Column 3: Data Points, Packet Drops, Out of Order
                        columns[2].vertical(|ui| {
                            ui.label(format!("Data Points: {}", stats.data_point_count));
                            ui.label(format!("Packet Drops: {:.1}%", stats.packet_drop_percentage));
                            ui.label(format!("Out of Order: {:.1}%", stats.out_of_order_percentage));
                        });
                    });
                } else if let Some(ref error) = self.load_error {
                    ui.colored_label(egui::Color32::RED, format!("Error: {error}"));
                } else if self.data_set.is_none() {
                    ui.label("No data loaded. Click 'Load Data' to import CSV file.");
                } else {
                    ui.label("No data selected. Use Shift+drag to select a range.");
                }
            })
    }

    // Helper method to handle plot selection
    fn handle_plot_selection(
        &mut self,
        ui: &egui::Ui,
        ctx: &egui::Context,
        response: &egui_plot::PlotResponse<()>,
        shift_pressed: bool,
    ) {
        if shift_pressed {
            let mouse_down = ui.input(|i| i.pointer.any_down());
            let mouse_pos = ui.input(|i| i.pointer.latest_pos());

            if let Some(_mouse_pos) = mouse_pos
                && let Some(plot_pos) = response.response.hover_pos()
            {
                let coord = response.transform.value_from_position(plot_pos);

                // Start selection on mouse press
                if mouse_down && !self.is_selecting {
                    self.selection_start = Some(coord.x);
                    self.is_selecting = true;
                }

                // Update selection on drag
                if self.is_selecting
                    && mouse_down
                    && let Some(start_x) = self.selection_start
                {
                    let (min_x, max_x) = if start_x < coord.x {
                        (start_x, coord.x)
                    } else {
                        (coord.x, start_x)
                    };
                    self.selected_x_range = Some((min_x, max_x));
                    ctx.request_repaint();
                }

                // End selection on mouse release
                if !mouse_down && self.is_selecting {
                    self.is_selecting = false;
                }
            }
        } else {
            // Reset selection state when Shift is not pressed
            if !self.is_selecting {
                self.selection_start = None;
            }
        }
    }

    fn export_selected_data(&mut self) {
        if let Some(selected_data) = self.get_selected_data() {
            // Open file dialog to choose save location
            if let Some(file_path) = rfd::FileDialog::new().add_filter("CSV files", &["csv"]).save_file() {
                match self.write_csv_data(&selected_data, &file_path) {
                    Ok(_) => {
                        self.load_error = Some(format!(
                            "Successfully exported {} data points to CSV",
                            selected_data.len()
                        ));
                    }
                    Err(e) => {
                        self.load_error = Some(format!("Failed to export CSV: {e}"));
                    }
                }
            }
        } else {
            self.load_error = Some("No data selected for export. Use Shift+drag to select a range first.".to_string());
        }
    }

    fn write_csv_data(&self, data: &[&crate::DataPoint], file_path: &std::path::Path) -> Result<(), anyhow::Error> {
        let file = std::fs::File::create(file_path)?;
        let mut writer = csv::Writer::from_writer(file);

        // Write header
        writer.write_record([
            "counter",
            "target_pps",
            "sender_achieved_pps",
            "receiver_calculated_pps",
            "latency_ms",
        ])?;

        // Write data points
        for point in data {
            writer.write_record(&[
                point.counter.to_string(),
                point.target_pps.to_string(),
                point.sender_achieved_pps.to_string(),
                point.receiver_calculated_pps.to_string(),
                point.latency_ms.to_string(),
            ])?;
        }

        writer.flush()?;
        Ok(())
    }
}

impl eframe::App for Inspector {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle keyboard shortcuts
        ctx.input_mut(|i| {
            // Handle Ctrl/Cmd + O for opening files
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O)) {
                self.load_data();
            }
            // Handle Ctrl/Cmd + E for CSV export
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::E)) {
                self.export_selected_data();
            }
        });

        // Add file loading controls at the top
        egui::TopBottomPanel::top("controls").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open (Ctrl+O)").clicked() {
                        self.load_data();
                    }
                    ui.separator();
                    if ui.button("Export CSV (Ctrl+E)").clicked() {
                        self.export_selected_data();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            if let Some(ref error) = self.load_error {
                ui.colored_label(egui::Color32::RED, format!("Error: {error}"));
                return;
            }

            ui.vertical(|ui| {
                let status = if self.data_set.is_some() {
                    self.render_collapsible_statistics(ui);
                    "Data loaded successfully"
                } else {
                    ""
                };
                ui.horizontal(|ui| {
                    ui.label(status);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some((min_x, max_x)) = self.selected_x_range {
                            ui.label(format!("Selection range: {} to {}", min_x as i32, max_x as i32));
                        }
                    })
                })
            });
        });

        // Main content area with 2x2 grid for plots
        egui::CentralPanel::default().show(ctx, |ui| {
            let available_size = ui.available_size();
            let grid_spacing = 10.0;
            let plot_area_height = available_size.y - grid_spacing * 3.0;
            let plot_area_width = available_size.x - grid_spacing * 3.0;

            // Create vertical layout
            ui.vertical(|ui| {
                // Plots section (takes remaining space after statistics)
                ui.allocate_ui(egui::vec2(plot_area_width, plot_area_height), |ui| {
                    let plot_height = (plot_area_height - grid_spacing) / 2.0;
                    let plot_width = (plot_area_width - grid_spacing) / 2.0;

                    // First row: PPS Plot and Latency Histogram
                    ui.horizontal(|ui| {
                        // PPS Plot (top-left)
                        ui.vertical(|ui| {
                            ui.heading("PPS");
                            ui.add_space(grid_spacing);
                            ui.allocate_ui(egui::vec2(plot_width, plot_height), |ui| {
                                self.render_pps_plot(ui, ctx);
                            });
                        });

                        ui.add_space(grid_spacing);

                        // Latency Histogram (top-right)
                        ui.vertical(|ui| {
                            ui.heading("Latency Histogram");
                            ui.add_space(grid_spacing);
                            ui.allocate_ui(egui::vec2(plot_width, plot_height), |ui| {
                                self.render_histogram_plot(ui);
                            });
                        });
                    });

                    // Second row: Latency vs Counter and Latency vs Receiver PPS
                    ui.horizontal(|ui| {
                        // Latency vs Counter (bottom-left)
                        ui.vertical(|ui| {
                            ui.heading("Latency");
                            ui.add_space(grid_spacing);
                            ui.allocate_ui(egui::vec2(plot_width, plot_height), |ui| {
                                self.render_latency_plot(ui, ctx);
                            });
                        });

                        ui.add_space(grid_spacing);

                        // Latency vs Receiver PPS (bottom-right)
                        ui.vertical(|ui| {
                            ui.heading("Latency vs Receiver PPS");
                            ui.add_space(grid_spacing);
                            ui.allocate_ui(egui::vec2(plot_width, plot_height), |ui| {
                                self.render_scatter_plot(ui);
                            });
                        });
                    });

                    ui.add_space(grid_spacing * 50.0);
                });
            });
        });
    }
}
