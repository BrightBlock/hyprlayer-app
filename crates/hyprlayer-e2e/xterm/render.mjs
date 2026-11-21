// A desktop's xterm for the e2e tests: @xterm/headless at the app's xterm version and buffer
// options (src/lib/terminal/terminalEngine.ts). One JSON command per stdin line, in order:
// {"resize":[cols,rows]}, {"write":"<base64>"}, {"dump":true}. A dump prints one JSON line.
import { createInterface } from "node:readline";
import headless from "@xterm/headless";

const term = new headless.Terminal({
  cols: 80,
  rows: 24,
  scrollback: 5000,
  convertEol: false,
  allowProposedApi: true,
});
const written = (data) => new Promise((done) => term.write(data, done));

function color(cell, fg) {
  if (fg ? cell.isFgDefault() : cell.isBgDefault()) return "d";
  const value = fg ? cell.getFgColor() : cell.getBgColor();
  if (fg ? cell.isFgRGB() : cell.isBgRGB()) return "#" + value.toString(16).padStart(6, "0");
  return "p" + value;
}

function flags(cell) {
  return (
    (cell.isBold() ? 1 : 0) |
    (cell.isDim() ? 2 : 0) |
    (cell.isItalic() ? 4 : 0) |
    (cell.isUnderline() ? 8 : 0) |
    (cell.isInverse() ? 16 : 0) |
    (cell.isStrikethrough() ? 32 : 0) |
    (cell.isInvisible() ? 64 : 0) |
    (cell.isBlink() ? 128 : 0)
  );
}

function dump() {
  const buffer = term.buffer.active;
  const cell = buffer.getNullCell();
  const grid = [];
  for (let y = 0; y < term.rows; y++) {
    const line = buffer.getLine(buffer.baseY + y);
    const row = [];
    for (let x = 0; x < term.cols; x++) {
      line.getCell(x, cell);
      row.push([cell.getChars(), cell.getWidth(), color(cell, true), color(cell, false), flags(cell)]);
    }
    grid.push(row);
  }
  const history = [];
  const wrapped = [];
  for (let y = 0; y < buffer.length; y++) {
    const line = buffer.getLine(y);
    if (y < buffer.baseY) history.push(line.translateToString(true).trimEnd());
    wrapped.push(line.isWrapped);
  }
  return {
    cols: term.cols,
    rows: term.rows,
    cursor: [buffer.cursorX, buffer.cursorY],
    alt: buffer.type === "alternate",
    grid,
    history,
    wrapped,
  };
}

let queue = Promise.resolve();
createInterface({ input: process.stdin }).on("line", (line) => {
  const command = JSON.parse(line);
  queue = queue.then(async () => {
    if (command.resize) term.resize(command.resize[0], command.resize[1]);
    if (command.write) await written(Buffer.from(command.write, "base64"));
    if (command.dump) process.stdout.write(JSON.stringify(dump()) + "\n");
  });
});
