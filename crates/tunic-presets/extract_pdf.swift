// Offline build tooling; PDFKit is supplied by macOS.
import Foundation
import PDFKit

let directory = URL(fileURLWithPath: CommandLine.arguments[1])
let files = try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)
for file in files where file.pathExtension == "pdf" {
    let output = file.deletingPathExtension().appendingPathExtension("txt")
    guard let document = PDFDocument(url: file), let text = document.string else { continue }
    try text.write(to: output, atomically: true, encoding: .utf8)
}
