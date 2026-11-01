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
    /// Turn a watch's crown: 1 up, -1 down.
    case crown(Int)
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
/// - On a watch, left and right bracket turn the crown up and down.
/// - Question mark reads these keys out.
///
/// Letters and question mark are the keys that type them on the Mac's
/// layout (see `useLayout`), so W is W on a French keyboard too. Arrows,
/// Tab, Space, Return and the digits 2 to 4 are where they are on every
/// keyboard. The crown keys are the brackets, or on layouts that need
/// Option for them, such as French, the two keys where US brackets are.
@MainActor
final class GestureKeys {
    var act: (GestureAction) -> Void = { _ in }

    var helpText: String { "Gesture keys. Arrows swipe. Hold one arrow and press another for a two-part swipe, such as up then left. Space double taps. T taps, R triple taps. H double taps and holds, and L touches and holds, until you let go. Hold 2, 3 or 4 while pressing a key to use that many fingers. Gestures happen at the touch point: Tab and Shift Tab move it from item to item, Shift arrows move it a step, C puts it in the middle, W says what's there and where, and D reads its properties. \(crownHelp)\(ReturnShortcut.current.spoken) returns to the Mac." }

    /// Keys that tap, hold or do something else, for the layout in use.
    private var taps = GestureKeys.fixedTaps
    private var holds: [UInt16: String] = [:]
    private var otherKeys: [UInt16: GestureAction] = [:]
    /// What the help says about the crown keys, ending in a space, or nothing.
    private var crownHelp = ""

    init() {
        useLayout(nil)
    }

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
    private static let fixedTaps: [UInt16: String] = [
        0x31: "double-tap", // Space
        0x24: "double-tap", // Return
    ]
    private static let tab: UInt16 = 0x30

    /// What a key named by the character it types does.
    private enum Role {
        case tap(String)
        /// Holds a touch down for as long as the key is held.
        case hold(String)
        case action(GestureAction)
    }

    /// Keys named by the character they type, with where that key is on a
    /// US keyboard, used when no key types it.
    private static let namedKeys: [(character: String, usKey: UInt16, role: Role)] = [
        ("t", 0x11, .tap("tap")),
        ("r", 0x0F, .tap("triple-tap")),
        ("h", 0x04, .hold("double-tap-hold")),
        ("l", 0x25, .hold("tap-hold")),
        ("c", 0x08, .action(.centre)),
        ("w", 0x0D, .action(.whereIsIt)),
        ("d", 0x02, .action(.details)),
        ("?", 0x2C, .action(.help)),
        ("[", 0x21, .action(.crown(1))),
        ("]", 0x1E, .action(.crown(-1))),
    ]

    /// Key codes only JIS keyboards have: yen, underscore, keypad comma,
    /// eisu and kana.
    private static let jisOnly: Set<UInt16> = [0x5D, 0x5E, 0x5F, 0x66, 0x68]

    /// Finds the gesture keys on a keyboard layout: `typing` says what a
    /// key types, with Shift or without (see `MacKeyboard.typing`). Without
    /// a layout, every key is where it is on a US keyboard. JIS-only keys
    /// count only on a JIS keyboard, as other keyboards don't have them.
    func useLayout(_ typing: ((UInt16, Bool) -> String?)?, jis: Bool = MacKeyboard.isJIS) {
        var taken = Set(Self.arrows.keys).union(Self.fingerKeys.keys).union(Self.modifiers)
            .union(Self.fixedTaps.keys).union([Self.tab])
        var found: [Int: UInt16] = [:]
        // Characters typed without Shift come first, so T is the key that
        // types t, not one that types T only with Shift.
        if let typing {
            for shifted in [false, true] {
                for code in UInt16(0)..<0x80 where !taken.contains(code) && (jis || !Self.jisOnly.contains(code)) {
                    guard let typed = typing(code, shifted)?.lowercased(),
                          let index = Self.namedKeys.firstIndex(where: { $0.character == typed }),
                          found[index] == nil
                    else { continue }
                    found[index] = code
                    taken.insert(code)
                }
            }
        }
        for (index, key) in Self.namedKeys.enumerated() where found[index] == nil && !taken.contains(key.usKey) {
            found[index] = key.usKey
            taken.insert(key.usKey)
        }
        taps = Self.fixedTaps
        holds = [:]
        otherKeys = [:]
        for (index, code) in found {
            switch Self.namedKeys[index].role {
            case let .tap(name): taps[code] = name
            case let .hold(name): holds[code] = name
            case let .action(action): otherKeys[code] = action
            }
        }
        let up = otherKeys.first { $0.value == .crown(1) }?.key
        let down = otherKeys.first { $0.value == .crown(-1) }?.key
        guard let up, let down else {
            crownHelp = ""
            return
        }
        if typing?(up, false) == "[" || typing?(up, true) == "[" || typing == nil {
            crownHelp = "On a watch, left bracket turns the crown up and right bracket turns it down. "
        } else {
            // Named by what they type, such as circumflex and dollar on a
            // French keyboard, or else by where they are.
            func name(_ code: UInt16, usKey: UInt16, position: String) -> String {
                let typed = typing?(code, false)
                if let typed, let name = Self.keyName(typed) {
                    return name
                }
                if code == usKey || typed == nil {
                    return position
                }
                return "the \(typed!) key"
            }
            let first = name(up, usKey: 0x21, position: "the first key after P")
            let second = name(down, usKey: 0x1E, position: "the second key after P")
            crownHelp = "On a watch, \(first) turns the crown up and \(second) turns it down. "
        }
    }

    /// A key's name as it's said, such as "the circumflex key" or "the
    /// semicolon key", in words, as screen readers may not read punctuation
    /// at their usual level. Nil for a character with no name here, which
    /// is then named by where the key is.
    private static func keyName(_ typed: String) -> String? {
        let names = ["^": "circumflex", "$": "dollar", "¨": "diaeresis", "*": "asterisk", "+": "plus",
                     "´": "acute accent", "`": "grave accent", "ü": "u umlaut", "è": "e grave", "ç": "c cedilla",
                     "à": "a grave", "é": "e acute", "º": "ordinal", "¡": "inverted exclamation mark",
                     "[": "left bracket", "]": "right bracket", ";": "semicolon", ":": "colon",
                     ",": "comma", ".": "period", "/": "slash", "\\": "backslash", "-": "minus", "=": "equals",
                     "'": "apostrophe", "\"": "quotation mark", "§": "section", "<": "less than",
                     ">": "greater than", "#": "number sign", "@": "at sign", "&": "ampersand", "%": "percent",
                     "!": "exclamation mark", "~": "tilde", "|": "vertical bar", "°": "degree", "£": "pound",
                     "€": "euro", "_": "underscore", "(": "left parenthesis", ")": "right parenthesis"]
        if let name = names[typed] {
            return "the \(name) key"
        }
        guard !typed.isEmpty, typed.allSatisfy({ $0.isLetter || $0.isNumber }) else { return nil }
        return "the \(typed) key"
    }

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
        if let tap = taps[code] {
            act(.gesture(prefix(fingers) + tap))
        } else if let hold = holds[code] {
            holding = code
            act(.press(prefix(fingers) + hold))
        } else if code == Self.tab {
            act(shift ? .previousItem : .nextItem)
        } else if let action = otherKeys[code] {
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
