use std::cmp;
use unicode_width::UnicodeWidthStr;
use zi::{
    components::border::{Border, BorderProperties},
    BindingMatch, BindingTransition, Canvas, Colour, Component, ComponentExt, ComponentLink, Key,
    Layout, Rect, ShouldRender, Size, Style,
};
use zi_term::Result;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Theme {
    logo: Style,
    tagline: Style,
    credits: Style,
}

impl Default for Theme {
    fn default() -> Self {
        const DARK0_SOFT: Colour = Colour::rgb(50, 48, 47);
        const LIGHT2: Colour = Colour::rgb(213, 196, 161);
        const GRAY_245: Colour = Colour::rgb(146, 131, 116);
        const BRIGHT_BLUE: Colour = Colour::rgb(131, 165, 152);

        Self {
            logo: Style::normal(DARK0_SOFT, LIGHT2),
            tagline: Style::normal(DARK0_SOFT, BRIGHT_BLUE),
            credits: Style::normal(DARK0_SOFT, GRAY_245),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SplashProperties {
    theme: Theme,
    logo: String,
    tagline: String,
    credits: String,
    offset: usize,
}

fn text_block_size(text: &str) -> Size {
    let width = text.lines().map(UnicodeWidthStr::width).max().unwrap_or(0);
    let height = text.lines().count();
    Size::new(width, height)
}

#[derive(Debug)]
struct Splash {
    properties: SplashProperties,
    frame: Rect,
}

impl Component for Splash {
    type Message = usize;
    type Properties = SplashProperties;

    fn create(properties: Self::Properties, frame: Rect, _link: ComponentLink<Self>) -> Self {
        Self { properties, frame }
    }

    fn change(&mut self, properties: Self::Properties) -> ShouldRender {
        if self.properties != properties {
            self.properties = properties;
            ShouldRender::Yes
        } else {
            ShouldRender::No
        }
    }

    fn resize(&mut self, frame: Rect) -> ShouldRender {
        self.frame = frame;
        ShouldRender::Yes
    }

    #[inline]
    fn view(&self) -> Layout {
        let logo_size = text_block_size(&self.properties.logo);
        let tagline_size = text_block_size(&self.properties.tagline);
        let credits_size = text_block_size(&self.properties.credits);

        let theme = Theme::default();
        let mut canvas = Canvas::new(self.frame.size);
        canvas.clear(theme.logo);

        // Draw logo
        let middle_x = (self.frame.size.width / 2).saturating_sub(logo_size.width / 2);
        let mut middle_y = cmp::min(8, self.frame.size.height.saturating_sub(logo_size.height))
            + self.properties.offset;
        for line in self.properties.logo.lines() {
            canvas.draw_str(middle_x, middle_y, theme.logo, line);
            middle_y += 1;
        }

        // Draw tagline
        middle_y += 2;
        let middle_x = (self.frame.size.width / 2).saturating_sub(tagline_size.width / 2);
        for line in self.properties.tagline.lines() {
            canvas.draw_str(middle_x, middle_y, theme.tagline, line);
            middle_y += 1;
        }

        // Draw credits
        middle_y += 1;
        let middle_x = (self.frame.size.width / 2).saturating_sub(credits_size.width / 2);
        for line in self.properties.credits.lines() {
            canvas.draw_str(middle_x, middle_y, theme.credits, line);
            middle_y += 1;
        }

        canvas.into()
    }
}

#[derive(Debug)]
struct SplashScreen {
    theme: Theme,
    link: ComponentLink<Self>,
}

impl Component for SplashScreen {
    type Message = usize;
    type Properties = ();

    fn create(_properties: Self::Properties, _frame: Rect, link: ComponentLink<Self>) -> Self {
        Self {
            theme: Default::default(),
            link,
        }
    }

    fn view(&self) -> Layout {
        // Instantiate our "splash screen" component
        let splash = Splash::with(SplashProperties {
            theme: self.theme.clone(),
            logo: SPLASH_LOGO.into(),
            tagline: SPLASH_TAGLINE.into(),
            credits: SPLASH_CREDITS.into(),
            offset: 0,
        });

        // Adding a border
        Border::with(BorderProperties::new(splash).style(self.theme.credits))
    }

    fn has_focus(&self) -> bool {
        true
    }

    fn input_binding(&self, pressed: &[Key]) -> BindingMatch<Self::Message> {
        let mut transition = BindingTransition::Clear;
        let message = match pressed {
            [Key::Ctrl('x'), Key::Ctrl('c')] => {
                self.link.exit();
                None
            }
            [Key::Ctrl('x')] => {
                transition = BindingTransition::Continue;
                None
            }
            _ => None,
        };
        BindingMatch {
            transition,
            message,
        }
    }
}

const SPLASH_LOGO: &str = r#"
   ▄████████    ▄███████▄  ▄█          ▄████████    ▄████████    ▄█    █▄
  ███    ███   ███    ███ ███         ███    ███   ███    ███   ███    ███
  ███    █▀    ███    ███ ███         ███    ███   ███    █▀    ███    ███
  ███          ███    ███ ███         ███    ███   ███         ▄███▄▄▄▄███▄▄
▀███████████ ▀█████████▀  ███       ▀███████████ ▀███████████ ▀▀███▀▀▀▀███▀
         ███   ███        ███         ███    ███          ███   ███    ███
   ▄█    ███   ███        ███▌    ▄   ███    ███    ▄█    ███   ███    ███
 ▄████████▀   ▄████▀      █████▄▄██   ███    █▀   ▄████████▀    ███    █▀
"#;
const SPLASH_TAGLINE: &str = "a splash screen for the terminal";
const SPLASH_CREDITS: &str = "C-x C-c to quit";

fn main() -> Result<()> {
    env_logger::init();
    zi_term::incremental()?.run_event_loop(SplashScreen::with(()))
}
