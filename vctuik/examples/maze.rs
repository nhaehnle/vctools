// SPDX-License-Identifier: GPL-3.0-or-later

use rand::prelude::*;

use ratatui::{layout::Rect, widgets::{Block, BorderType, Borders}};
use vctuik::{event::KeyCode, prelude::*, state::Builder};

const LEFT: u8 = 1;
const RIGHT: u8 = 2;
const DOWN: u8 = 4;
const UP: u8 = 8;

const CONNECTORS: [char; 16] = [
    ' ', '╴', '╶', '─', '╷', '┐', '┌', '┬', '╵', '┘', '└', '┴', '│', '┤', '├', '┼',
];

struct GameState {
    width: usize,
    height: usize,
    maze: Vec<u8>,
}
impl GameState {
    fn new(width: usize, height: usize) -> Self {
        let density: u8 = 140;
        let mut maze = vec![0; width * height];
        let mut rng = rand::thread_rng();
        for y in 0..height {
            for x in 0..width {
                if x + 1 < width && rng.gen::<u8>() < density {
                    maze[y * width + x] |= RIGHT;
                    maze[y * width + x + 1] |= LEFT;
                }
                if rng.gen::<u8>() < density {
                    maze[y * width + x] |= UP;
                    if y + 1 < height {
                        maze[(y + 1) * width + x] |= DOWN;
                    }
                }
            }
        }
        Self { width, height, maze }
    }
}

fn add_game_view(builder: &mut Builder, game_state: &mut GameState)
{
    let area = builder.take_lines_fixed(game_state.height as u16 + 2);

    let game_area = Rect {
        x: area.width.saturating_sub(game_state.width as u16) / 2 - 1,
        width: game_state.width as u16 + 2,
        ..area
    };
    let block = Block::new()
        .border_type(BorderType::Thick)
        .borders(Borders::ALL);
    builder.frame().render_widget(block, game_area);
    let buffer = builder.frame().buffer_mut();
    for y in 0..game_state.height {
        for x in 0..game_state.width {
            if let Some(cell) = buffer.cell_mut((game_area.x + (x + 1) as u16, game_area.y + (game_state.height - y) as u16)) {
                cell.set_char(CONNECTORS[game_state.maze[y * game_state.width + x] as usize]);
            }
        }
    }
}

fn main() -> Result<()> {
    let mut terminal = vctuik::init()?;

    let mut running = true;
    let mut game_state = GameState::new(10, 20);

    terminal.run(|builder| {
        if builder.on_key_press(KeyCode::Char('q')) {
            running = false;
            return Ok(false);
        }

        add_game_view(builder, &mut game_state);

        Ok(true)
    })?;

    Ok(())
}
