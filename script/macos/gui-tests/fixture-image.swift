import AppKit

// Preserve full-screen originals; reproduce window.screenshot() using the
// independent AX rectangle recorded beside each new fixture.
func fixtureImage(_ directory: URL, _ name: String) throws -> Data {
    let data = try Data(contentsOf: directory.appendingPathComponent(name))
    let entries = try ["provenance-137.json", "provenance-141.json"].flatMap { name -> [[String: Any]] in
        let manifest = try Data(contentsOf: directory.appendingPathComponent(name))
        return (try JSONSerialization.jsonObject(with: manifest) as! [String: Any])["images"] as! [[String: Any]]
    }
    guard let entry = entries.first(where: { $0["name"] as? String == name }) else { return data }
    let bounds = entry["window_crop_xyxy"] as! [Int]
    let rect = CGRect(x: CGFloat(bounds[0]), y: CGFloat(bounds[1]),
                      width: CGFloat(bounds[2] - bounds[0]), height: CGFloat(bounds[3] - bounds[1]))
    guard let original = NSBitmapImageRep(data: data)?.cgImage,
          let image = original.cropping(to: rect),
          let cropped = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else {
        throw NSError(domain: "ZaplexOCRFixture", code: 1)
    }
    return cropped
}
