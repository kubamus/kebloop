import SwiftUI

struct DictionaryLookupView: View {
    @State private var word: String
    @State private var definition: String?
    @State private var hasLookedUp = false

    init(word: String = "") { _word = State(initialValue: word) }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                TextField(L("Word or phrase"), text: $word)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(lookup)
                Button(L("Look Up"), action: lookup)
                    .keyboardShortcut(.return, modifiers: .command)
            }

            if let definition {
                ScrollView {
                    Text(definition)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .textSelection(.enabled)
                        .padding(12)
                }
                .background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
            } else if hasLookedUp {
                VStack(alignment: .leading, spacing: 8) {
                    Text(L("No definition was found in the active macOS dictionaries."))
                        .foregroundStyle(.secondary)
                    Button(L("Open in Dictionary")) { _ = DictionaryLookupService.openInDictionary(word) }
                }
            } else {
                Text(L("Look up a word in the dictionaries enabled in macOS."))
                    .foregroundStyle(.secondary)
            }
        }
        .padding(18)
        .frame(minWidth: 500, minHeight: 330)
        .onAppear { if !word.isEmpty { lookup() } }
    }

    private func lookup() {
        definition = DictionaryLookupService.definition(for: word)
        hasLookedUp = true
    }
}
