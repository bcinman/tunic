use std::{cell::Cell, rc::Rc};

use gpui::{
    Bounds, Div, Entity, PathBuilder, Pixels, Point, Window, canvas, div, fill, point, prelude::*,
    px, rgb, rgba, size,
};
use tunic_core::{
    Chain, FilterId, FrequencyHz, FrequencyResponse, ProcessorError, SPECTRUM_POINT_COUNT,
    SampleRateHz,
};

use super::telemetry::SpectrumView;

const MIN_FREQUENCY_HZ: f64 = 20.0;
const MAX_FREQUENCY_HZ: f64 = 20_000.0;
const MIN_GAIN_DB: f64 = -20.0;
const MAX_GAIN_DB: f64 = 20.0;
const POINT_RADIUS_PX: f32 = 4.0;
const POINT_INSET_PX: f32 = POINT_RADIUS_PX + 1.0;
const POINT_HIT_RADIUS_PX: f32 = 12.0;

pub(crate) fn graph(
    chain: &Chain,
    exposed: &[FilterId],
    sample_rate: SampleRateHz,
    spectrum: Entity<SpectrumView>,
    graph_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
) -> Div {
    let max_frequency = MAX_FREQUENCY_HZ.min(sample_rate.into_inner() * 0.499);
    let (response, response_error) = match response(chain, sample_rate, max_frequency) {
        Ok(response) => (response, None),
        Err(error) => (Vec::new(), Some(format!("Response unavailable: {error:?}"))),
    };
    let filter_responses = individual_responses(chain, sample_rate, max_frequency);
    let points = chain
        .equalizer
        .filters
        .iter()
        .map(|filter| {
            (
                frequency_fraction(filter.parameters.frequency.into_inner(), max_frequency),
                gain_fraction(filter.parameters.gain.into_inner()),
                exposed.contains(&filter.id),
            )
        })
        .collect::<Vec<_>>();
    let max_frequency_label = if max_frequency >= 1_000.0 {
        format!("{:.0}kHz", max_frequency / 1_000.0)
    } else {
        format!("{max_frequency:.0}Hz")
    };

    div()
        .relative()
        .h(px(250.0))
        .child(div().absolute().size_full().child(spectrum))
        .child(
            canvas(
                move |bounds, _, _| graph_bounds.set(Some(bounds)),
                move |bounds, _, window, _| {
                    let zero_y = bounds.origin.y + bounds.size.height * gain_fraction(0.0);
                    window.paint_quad(fill(
                        Bounds {
                            origin: point(bounds.origin.x, zero_y),
                            size: size(bounds.size.width, px(1.0)),
                        },
                        rgba(0xffffff1a),
                    ));

                    for filter_response in &filter_responses {
                        paint_response(bounds, filter_response, px(1.0), rgba(0xbfc2c533), window);
                    }
                    if response.iter().any(|gain| gain.abs() > f32::EPSILON) {
                        paint_response(bounds, &response, px(2.0), rgb(0xd8d8d8), window);
                    }

                    for (x, y, tweakable) in &points {
                        let radius = px(POINT_RADIUS_PX);
                        let center = graph_point(bounds, *x, *y);
                        window.paint_quad(fill(
                            Bounds {
                                origin: point(center.x - radius, center.y - radius),
                                size: size(radius * 2.0, radius * 2.0),
                            },
                            if *tweakable {
                                rgb(0xf2f2f2)
                            } else {
                                rgb(0x77777a)
                            },
                        ));
                    }
                },
            )
            .absolute()
            .size_full(),
        )
        .child(
            div()
                .absolute()
                .bottom_3()
                .left_3()
                .text_sm()
                .text_color(rgb(0x505054))
                .child("20Hz"),
        )
        .child(
            div()
                .absolute()
                .bottom_3()
                .right_3()
                .text_sm()
                .text_color(rgb(0x505054))
                .child(max_frequency_label),
        )
        .when_some(response_error, |graph, error| {
            graph.child(
                div()
                    .absolute()
                    .top_3()
                    .left_3()
                    .text_sm()
                    .text_color(rgb(0xff8a8a))
                    .child(error),
            )
        })
}

pub(crate) fn response(
    chain: &Chain,
    sample_rate: SampleRateHz,
    max_frequency: f64,
) -> Result<Vec<f32>, ProcessorError> {
    let prepared = FrequencyResponse::new(chain, sample_rate)?;
    (0..SPECTRUM_POINT_COUNT)
        .map(|index| {
            let fraction = index as f64 / (SPECTRUM_POINT_COUNT - 1) as f64;
            let frequency = MIN_FREQUENCY_HZ * (max_frequency / MIN_FREQUENCY_HZ).powf(fraction);
            let frequency = FrequencyHz::try_new(frequency).expect("graph frequency is positive");
            prepared.db_at(frequency).map(|response| response as f32)
        })
        .collect()
}

pub(crate) fn individual_responses(
    chain: &Chain,
    sample_rate: SampleRateHz,
    max_frequency: f64,
) -> Vec<Vec<f32>> {
    chain
        .equalizer
        .filters
        .iter()
        .filter_map(|filter| {
            let mut filter_chain = Chain::default();
            filter_chain.equalizer.filters.push(*filter);
            response(&filter_chain, sample_rate, max_frequency).ok()
        })
        .collect()
}

fn paint_response(
    bounds: Bounds<Pixels>,
    response: &[f32],
    stroke_width: Pixels,
    color: gpui::Rgba,
    window: &mut Window,
) {
    if response.is_empty() {
        return;
    }
    let mut path = PathBuilder::stroke(stroke_width);
    for (index, response) in response.iter().copied().enumerate() {
        let x = bounds.origin.x
            + bounds.size.width * (index as f32 / (SPECTRUM_POINT_COUNT - 1) as f32);
        let y = bounds.origin.y + bounds.size.height * gain_fraction(f64::from(response));
        if index == 0 {
            path.move_to(point(x, y));
        } else {
            path.line_to(point(x, y));
        }
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

pub(crate) fn frequency_fraction(frequency: f64, max_frequency: f64) -> f32 {
    ((frequency.clamp(MIN_FREQUENCY_HZ, max_frequency) / MIN_FREQUENCY_HZ).ln()
        / (max_frequency / MIN_FREQUENCY_HZ).ln()) as f32
}

pub(crate) fn gain_fraction(gain_db: f64) -> f32 {
    ((MAX_GAIN_DB - gain_db.clamp(MIN_GAIN_DB, MAX_GAIN_DB)) / (MAX_GAIN_DB - MIN_GAIN_DB)) as f32
}

pub(crate) fn frequency_at_fraction(fraction: f32, max_frequency: f64) -> f64 {
    MIN_FREQUENCY_HZ * (max_frequency / MIN_FREQUENCY_HZ).powf(f64::from(fraction.clamp(0.0, 1.0)))
}

pub(crate) fn gain_at_fraction(fraction: f32) -> f64 {
    MAX_GAIN_DB - f64::from(fraction.clamp(0.0, 1.0)) * (MAX_GAIN_DB - MIN_GAIN_DB)
}

pub(crate) fn graph_point(bounds: Bounds<Pixels>, x: f32, y: f32) -> Point<Pixels> {
    let inset = px(POINT_INSET_PX);
    point(
        bounds.origin.x + inset + (bounds.size.width - inset * 2.0) * x,
        bounds.origin.y + inset + (bounds.size.height - inset * 2.0) * y,
    )
}

fn graph_fractions(bounds: Bounds<Pixels>, position: Point<Pixels>) -> (f32, f32) {
    let inset = px(POINT_INSET_PX);
    let width = bounds.size.width - inset * 2.0;
    let height = bounds.size.height - inset * 2.0;
    (
        ((position.x - bounds.origin.x - inset) / width).clamp(0.0, 1.0),
        ((position.y - bounds.origin.y - inset) / height).clamp(0.0, 1.0),
    )
}

pub(crate) fn closest_filter(
    chain: &Chain,
    sample_rate: SampleRateHz,
    bounds: Bounds<Pixels>,
    position: Point<Pixels>,
) -> Option<FilterId> {
    let max_frequency = MAX_FREQUENCY_HZ.min(sample_rate.into_inner() * 0.499);
    chain
        .equalizer
        .filters
        .iter()
        .filter_map(|filter| {
            let center = graph_point(
                bounds,
                frequency_fraction(filter.parameters.frequency.into_inner(), max_frequency),
                gain_fraction(filter.parameters.gain.into_inner()),
            );
            let x = (position.x - center.x).as_f32();
            let y = (position.y - center.y).as_f32();
            let distance_squared = x.mul_add(x, y * y);
            (distance_squared <= POINT_HIT_RADIUS_PX.powi(2))
                .then_some((filter.id, distance_squared))
        })
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(id, _)| id)
}

pub(crate) fn values_at_position(
    bounds: Bounds<Pixels>,
    position: Point<Pixels>,
    max_frequency: f64,
) -> (f64, f64) {
    let (x, y) = graph_fractions(bounds, position);
    (frequency_at_fraction(x, max_frequency), gain_at_fraction(y))
}

pub(crate) const fn maximum_frequency_hz() -> f64 {
    MAX_FREQUENCY_HZ
}
