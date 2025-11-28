use std::io::{self, Write};
use streamtify::{Message, PlayerCommand};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. ถามชื่อห้อง (Login)
    print!("🔑 Enter Room Name (e.g. MyRoom, Party): ");
    io::stdout().flush()?;
    let mut room_name = String::new();
    std::io::stdin().read_line(&mut room_name)?;
    let room_name = room_name.trim();

    // 2. Connect
    let mut socket = TcpStream::connect("127.0.0.1:9000").await?;
    println!("✅ Connected! Joining room: {}", room_name);

    // 3. ส่งชื่อห้องไปบอก Server
    let (reader, mut writer) = socket.split();
    writer.write_all(room_name.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    let mut reader = BufReader::new(reader);
    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());

    loop {
        let mut line = String::new();
        let mut input = String::new();

        tokio::select! {
            // A. รับ State มาแสดงผล
            Ok(bytes) = reader.read_line(&mut line) => {
                if bytes == 0 { break; }
                if let Ok(Message::StateUpdate(s)) = serde_json::from_str::<Message>(&line) {
                    // Clear Screen
                    print!("\x1B[2J\x1B[1;1H");

                    println!("╔═══════════════════════════════════════╗");
                    println!("║ 🏠 ROOM: {:<29}║", room_name);
                    println!("╠═══════════════════════════════════════╣");
                    println!("║ 🎵 Song:   {:<27}║", s.song_title);
                    println!("║ 🎤 Artist: {:<27}║", s.artist);
                    println!("║ 💿 Track:  {}/{}                      ║", s.current_index + 1, s.total_songs);
                    println!("║ ▶️  State:  {:<27}║", if s.is_playing { "PLAYING 🔊" } else { "PAUSED 🔇" });

                    // Progress Bar
                    let pct = (s.progress_ms as f64 / s.duration_ms as f64) * 30.0;
                    let bar_len = pct as usize;
                    let bar = "█".repeat(bar_len);
                    let space = "-".repeat(if bar_len > 30 { 0 } else { 30 - bar_len });

                    let cm = s.progress_ms / 60000;
                    let cs = (s.progress_ms % 60000) / 1000;
                    let tm = s.duration_ms / 60000;
                    let ts = (s.duration_ms % 60000) / 1000;

                    println!("║ ⏳ [{}{}] {:02}:{:02}/{:02}:{:02} ║", bar, space, cm, cs, tm, ts);
                    println!("╚═══════════════════════════════════════╝");
                    println!("Controls: [p] Play  [s] Stop/Pause  [n] Next  [b] Back  [r] Restart");
                }
                line.clear();
            }

            // B. รับ Input ส่งคำสั่ง
            Ok(_) = stdin.read_line(&mut input) => {
                let cmd = match input.trim() {
                    "p" => Some(PlayerCommand::Play),
                    "s" => Some(PlayerCommand::Pause),
                    "n" => Some(PlayerCommand::Next),
                    "b" => Some(PlayerCommand::Prev),
                    "r" => Some(PlayerCommand::Restart),
                    _ => None,
                };

                if let Some(c) = cmd {
                    let msg = Message::Command(c);
                    let json = serde_json::to_string(&msg).unwrap();
                    writer.write_all(json.as_bytes()).await?;
                    writer.write_all(b"\n").await?;
                }
                input.clear();
            }
        }
    }
    Ok(())
}
