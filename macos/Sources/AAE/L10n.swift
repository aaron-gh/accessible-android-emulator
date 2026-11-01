import Foundation

/// AAE in the user's language.
///
/// SwiftUI translates the labels written in the views, such as
/// `Button("Cold Boot")`, from `fr.lproj/Localizable.strings`, which build.sh
/// makes from `translations/fr.po`. Everything else, from announcements built
/// here to messages from the Rust core, is English text translated on its way
/// out with `tr`, from the same catalog (see aae_core::l10n).
enum L10n {
    /// Uses the Mac's first preferred language, if AAE has it.
    static func start() {
        _ = setLanguage(tag: Locale.preferredLanguages.first ?? "en")
    }
}

/// English text in the language in use.
func tr(_ text: String) -> String {
    translate(text: text)
}

/// As `tr`, for a word with more than one meaning in English, such as
/// "Watch": the translation for this use.
func tr(_ text: String, context: String) -> String {
    translateIn(context: context, text: text)
}
