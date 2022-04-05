use num_complex::Complex;
use rayon::{iter::ParallelExtend, prelude::*};
use zi::{prelude::*, terminal::SquarePixelGrid};
use zi_term::Result;

type Position = euclid::default::Point2D<f64>;

#[derive(Clone, Debug, Default, PartialEq)]
struct Properties {
    position: Position,
    scale: f64,
}

#[derive(Debug)]
struct Mandelbrot {
    properties: Properties,
    frame: Rect,
    fractal: Vec<(usize, usize, f64)>,
    min: f64,
    max: f64,
}

impl Mandelbrot {
    fn compute_fractal(&mut self, size: Size) {
        let Self {
            properties: Properties { position, scale },
            ..
        } = *self;

        let width = size.width as f64;
        let height = size.height as f64;

        self.fractal.clear();
        self.fractal
            .par_extend((0..size.width).into_par_iter().flat_map(|x| {
                (0..size.height).into_par_iter().map(move |y| {
                    let xf = (x as f64 - width / 2.0) * scale + position.x;
                    let yf = (y as f64 - height / 2.0) * scale + position.y;
                    let c = Complex::new(xf, yf);
                    let mut z = Complex::new(0.0, 0.0);
                    let target = 4.0;
                    let mut num_steps = 0;
                    for _ in 0..1000 {
                        num_steps += 1;
                        z = z * z + c;
                        if z.norm_sqr() > target {
                            break;
                        }
                    }
                    let conv = (num_steps as f64 / 1000.0).max(0.0).min(1.0);
                    // let conv2 = 1.0 - (z.norm_sqr() / target).max(0.0).min(1.0);
                    // let conv = conv1 * conv2;
                    // let xx = (conv * 255.0).floor() as u8;
                    // let g = colorous::CUBEHELIX.eval_continuous(1.0 - conv);
                    // Colour::rgb(g.r, g.g, g.b)

                    (x, y, conv)
                })
            }));
        self.min = self
            .fractal
            .par_iter()
            .cloned()
            .reduce(|| (0, 0, 1.0), |x, y| (0, 0, x.2.min(y.2)))
            .2;
        self.max = self
            .fractal
            .par_iter()
            .cloned()
            .reduce(|| (0, 0, 0.0), |x, y| (0, 0, x.2.max(y.2)))
            .2;
    }
}
impl Component for Mandelbrot {
    type Message = ();
    type Properties = Properties;

    fn create(properties: Self::Properties, frame: Rect, _link: ComponentLink<Self>) -> Self {
        let mut component = Self {
            properties,
            frame,
            fractal: Vec::new(),
            min: 0.0,
            max: 0.0,
        };
        component.compute_fractal(Size::new(frame.size.width, 2 * frame.size.height));
        component
    }

    fn change(&mut self, properties: Self::Properties) -> ShouldRender {
        if self.properties != properties {
            self.properties = properties;
            self.compute_fractal(Size::new(self.frame.size.width, 2 * self.frame.size.height));
            ShouldRender::Yes
        } else {
            ShouldRender::No
        }
    }

    fn resize(&mut self, frame: Rect) -> ShouldRender {
        self.frame = frame;
        self.compute_fractal(Size::new(self.frame.size.width, 2 * self.frame.size.height));
        ShouldRender::Yes
    }

    #[inline]
    fn view(&self) -> Layout {
        // eprintln!("Range: {} -> {}", self.min, self.max);
        let mut grid = SquarePixelGrid::from_available(self.frame.size);
        for (x, y, conv) in self.fractal.iter() {
            // let g = colorous::CUBEHELIX.eval_continuous(1.0 - conv);
            let g = colorous::CUBEHELIX
                .eval_continuous(1.0 - (conv - self.min) / (self.max - self.min));
            grid.draw(zi::Position::new(*x, *y), Colour::rgb(g.r, g.g, g.b));
        }
        grid.into_canvas().into()
    }
}

enum Message {
    MoveUp,
    MoveRight,
    MoveDown,
    MoveLeft,
    ZoomIn,
    ZoomOut,
}

#[derive(Debug)]
struct Viewer {
    position: Position,
    scale: f64,
    link: ComponentLink<Self>,
}

impl Component for Viewer {
    type Message = Message;
    type Properties = ();

    fn create(_properties: Self::Properties, _frame: Rect, link: ComponentLink<Self>) -> Self {
        Self {
            position: Position::new(-1.0, -1.0),
            scale: 0.01,
            link,
        }
    }

    fn update(&mut self, message: Self::Message) -> ShouldRender {
        let step = self.scale * 2.0;
        match message {
            Message::MoveUp => self.position.y -= step,
            Message::MoveDown => self.position.y += step,
            Message::MoveLeft => self.position.x -= step,
            Message::MoveRight => self.position.x += step,
            Message::ZoomIn => self.scale /= 1.05,
            Message::ZoomOut => self.scale *= 1.05,
        }
        ShouldRender::Yes
    }

    fn change(&mut self, _properties: Self::Properties) -> ShouldRender {
        ShouldRender::Yes
    }

    fn view(&self) -> Layout {
        Mandelbrot::with(Properties {
            position: self.position,
            scale: self.scale,
        })
    }

    fn bindings(&self, bindings: &mut Bindings<Self>) {
        // If we already initialised the bindings, nothing to do -- they never
        // change in this example
        if !bindings.is_empty() {
            return;
        }
        // Set focus to `true` in order to react to key presses
        bindings.set_focus(true);

        // Panning
        bindings.add("move-up", [Key::Char('w')], || Message::MoveUp);
        bindings.add("move-right", [Key::Char('d')], || Message::MoveRight);
        bindings.add("move-down", [Key::Char('s')], || Message::MoveDown);
        bindings.add("move-left", [Key::Char('a')], || Message::MoveLeft);

        // Zoom
        bindings.add("zoom-in", [Key::Char('=')], || Message::ZoomIn);
        bindings.add("zoom-out", [Key::Char('-')], || Message::ZoomOut);

        // Exit
        bindings.add("exit", [Key::Ctrl('x'), Key::Ctrl('c')], |this: &Self| {
            this.link.exit()
        });
    }
}

fn main() -> Result<()> {
    env_logger::init();
    zi_term::incremental()?.run_event_loop(Viewer::with(()))
}
