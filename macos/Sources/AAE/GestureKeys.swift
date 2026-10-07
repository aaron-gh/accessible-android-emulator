import Foundation

/// Something a key does in gesture mode.
enum GestureAction: Equatable {
    /// Perform a gesture, such as "swipe-up-then-left", at the touch point.
    case gesture(String)
    /// Perform a gesture and leave its last touch down, until `.release`.
    case press(String)
    case release
    /// Move the touch point to the next or previous thing on the screen.
    case nextItem, previousItem
    /// Move the touch point a step: -1, 0 or 1 across and down.
    case step(dx: Int, dy: Int)
    /// Put the touch point back in the middle of the screen.
    case centre
    /// Say what's at the touch point, and where.
    case whereIsIt
    /// Read every property of the element at the touch point.
    case details
    case help
    /// A key that does nothing in gesture mode.
    case unknown
}

/// Turns keys into gestures, in gesture mode. Every key is free, so no
/// modifiers are needed. Gestures happen at the touch point, which starts in
/// the middle of the screen:
///
/// - An arrow swipes that way when released. Holding one arrow and pressing
///   another performs a two-part swipe, such as up then left, at once.
/// - Space or Return double taps, T taps, and R triple taps. H double
///   taps and holds, and L touches and holds, for as long as the key is held.
/// - Holding 2, 3 or 4 while pressing an arrow or a tap key uses that many
///   fingers.
/// - Tab and Shift-Tab move the touch point to the next or previous thing on
///   the screen; Shift-arrows move it a step. C puts it back in the middle,
///   W says what's there and where, and D reads its properties.
/// - Question mark reads these keys out.
@MainActor
final class GestureKeys {
    var act: (GestureAction) -> Void = { _ in }

    static var helpText: String { "Gesture keys. Arrows swipe. Hold one arrow and press another for a two-part swipe, such as up then left. Space double taps. T taps, R triple taps. H double taps and holds, and L touches and holds, until you let go. Hold 2, 3 or 4 while pressing a key to use that many fingers. Gestures happen at the touch point: Tab and Shift Tab move it from item to item, Shift arrows move it a step, C puts it in the middle, W says what's there and where, and D reads its properties. \(ReturnShortcut.current.spoken) returns to the Mac." }

    /// Keys down, as far as gesture mode knows. A key going down again while
    /// in here is the keyboard repeating it.
    private var down = Set<UInt16>()
    /// An arrow pressed and not yet released: a swipe that may become the
    /// first part of a two-part swipe.
    private var pending: (key: UInt16, direction: String, fingers: Int)?
    /// The key holding a touch down, if one is.
    private var holding: UInt16?

    private static let arrows: [UInt16: (name: String, dx: Int, dy: Int)] = [
        0x7B: ("left", -1, 0), 0x7C: ("right", 1, 0), 0x7D: ("down", 0, 1), 0x7E: ("up", 0, -1),
    ]
    private static let fingerKeys: [UInt16: Int] = [0x12: 1, 0x13: 2, 0x14: 3, 0x15: 4]
    private static let shiftKeys: Set<UInt16> = [0x38, 0x3C]
    private static let modifiers: Set<UInt16> = [0x36, 0x37, 0x38, 0x3C, 0x3B, 0x3E, 0x3A, 0x3D, 0x39, 0x3F]
    private static let taps: [UInt16: String] = [
        0x31: "double-tap", // Space
        0x24: "double-tap", // Return
        0x11: "tap", // T
        0x0F: "triple-tap", // R
    ]
    /// Keys that hold a touch down for as long as they're held.
    private static let holds: [UInt16: String] = [
        0x04: "double-tap-hold", // H
        0x25: "tap-hold", // L
    ]
    private static let otherKeys: [UInt16: GestureAction] = [
        0x08: .centre, // C
        0x0D: .whereIsIt, // W
        0x02: .details, // D
        0x2C: .help, // slash, for question mark
    ]
    private static let tab: UInt16 = 0x30

    /// Handles a key going down or up.
    ///
    /// VoiceOver swallows the release of the first of two arrows pressed
    /// together, and macOS then marks that arrow's next presses as repeats.
    /// So nothing here waits for that release, or trusts macOS's repeat flag:
    /// a two-part swipe happens as soon as its second arrow goes down, and
    /// its first arrow then counts as released.
    func handle(_ code: UInt16, _ isDown: Bool) {
        if KeyLog.enabled {
            KeyLog.write("gesture key \(code) \(isDown ? "down" : "up") pending=\(pending?.direction ?? "none")")
        }
        guard isDown else {
            down.remove(code)
            if code == holding {
                lift()
            } else if let swipe = pending, swipe.key == code {
                // Releasing the arrow of a swipe still waiting makes it a plain swipe.
                pending = nil
                act(.gesture(prefix(swipe.fingers) + "swipe-" + swipe.direction))
            }
            return
        }
        guard down.insert(code).inserted else { return }
        if Self.modifiers.contains(code) || Self.fingerKeys[code] != nil { return }
        // Another key lifts a held touch, in case its key's release never came.
        lift()
        if let arrow = Self.arrows[code] {
            if shift {
                finishPending()
                act(.step(dx: arrow.dx, dy: arrow.dy))
                return
            }
            if let first = pending {
                pending = nil
                if first.fingers == 1, fingers == 1, first.direction != arrow.name {
                    down.remove(first.key)
                    act(.gesture("swipe-\(first.direction)-then-\(arrow.name)"))
                    return
                }
                // Not a two-part swipe: do the first, and wait on this one.
                act(.gesture(prefix(first.fingers) + "swipe-" + first.direction))
            }
            pending = (code, arrow.name, fingers)
            return
        }
        finishPending()
        if let tap = Self.taps[code] {
            act(.gesture(prefix(fingers) + tap))
        } else if let hold = Self.holds[code] {
            holding = code
            act(.press(prefix(fingers) + hold))
        } else if code == Self.tab {
            act(shift ? .previousItem : .nextItem)
        } else if let action = Self.otherKeys[code] {
            act(action)
        } else {
            act(.unknown)
        }
    }

    /// Forgets keys and half-drawn swipes, lifting any held touch, when
    /// gesture mode ends.
    func reset() {
        lift()
        down.removeAll()
        pending = nil
    }

    private func lift() {
        guard holding != nil else { return }
        holding = nil
        act(.release)
    }

    /// Performs a swipe still waiting on its arrow's release, before another
    /// key does something else.
    private func finishPending() {
        guard let swipe = pending else { return }
        pending = nil
        act(.gesture(prefix(swipe.fingers) + "swipe-" + swipe.direction))
    }

    private var shift: Bool {
        !down.isDisjoint(with: Self.shiftKeys)
    }

    /// The number of fingers: the largest of 2, 3 or 4 held down, otherwise one.
    private var fingers: Int {
        down.compactMap { Self.fingerKeys[$0] }.max() ?? 1
    }

    private func prefix(_ fingers: Int) -> String {
        fingers > 1 ? "\(["", "one", "two", "three", "four"][fingers])-finger-" : ""
    }
}
