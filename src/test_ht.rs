
use transmute::html_to_text;
fn main() {
    let html = "<p>line1<br/>line2</p><p>line3</p>";
    let text = html_to_text(html);
    println!("input:  {}", html.escape_debug());
    println!("output: {}", text.escape_debug());
}
