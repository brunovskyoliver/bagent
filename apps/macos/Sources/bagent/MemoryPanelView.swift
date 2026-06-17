import SwiftUI

struct MemoryPanelView: View {
    @ObservedObject var viewModel: ChatViewModel

    private let kindFilters: [(label: String, kind: String)] = [
        ("Všetko", ""),
        ("Preferencie", "preference"),
        ("Opravy", "correction"),
        ("Glosár SK", "sk_glossary"),
        ("Fakty", "fact"),
        ("Entity", "entity"),
    ]

    var body: some View {
        VStack(spacing: 0) {
            searchBar
            kindChips
            Divider()
            itemList
        }
        .background(.ultraThinMaterial)
        .onChange(of: viewModel.memorySearchQuery) { _, q in
            Task { await viewModel.searchMemory(query: q) }
        }
        .onChange(of: viewModel.memoryKindFilter) { _, _ in
            viewModel.applyMemoryFilter()
        }
    }

    private var searchBar: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
                .font(.system(size: 11))
            TextField("Hľadaj v pamäti…", text: $viewModel.memorySearchQuery)
                .font(.system(size: 12))
                .textFieldStyle(.plain)
            if !viewModel.memorySearchQuery.isEmpty {
                Button { viewModel.memorySearchQuery = "" } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundStyle(.secondary)
                        .font(.system(size: 10))
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
    }

    private var kindChips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 5) {
                ForEach(kindFilters, id: \.kind) { filter in
                    Button {
                        viewModel.memoryKindFilter = filter.kind
                    } label: {
                        Text(filter.label)
                            .font(.system(size: 10, weight: .medium))
                            .padding(.horizontal, 8)
                            .padding(.vertical, 3)
                            .background(
                                viewModel.memoryKindFilter == filter.kind
                                    ? Color.accentColor.opacity(0.85)
                                    : Color.secondary.opacity(0.15)
                            )
                            .foregroundStyle(
                                viewModel.memoryKindFilter == filter.kind
                                    ? Color.white
                                    : Color.primary
                            )
                            .clipShape(Capsule())
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
        }
    }

    @ViewBuilder
    private var itemList: some View {
        if viewModel.isLoadingMemory {
            VStack {
                Spacer()
                ProgressView()
                    .scaleEffect(0.7)
                Text("Načítavam…")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                Spacer()
            }
            .frame(maxWidth: .infinity)
        } else if viewModel.filteredMemoryItems.isEmpty {
            VStack(spacing: 6) {
                Spacer()
                Image(systemName: "brain")
                    .font(.system(size: 24))
                    .foregroundStyle(.tertiary)
                Text(viewModel.memorySearchQuery.isEmpty ? "Žiadne záznamy v pamäti." : "Žiadne výsledky.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                Spacer()
            }
            .frame(maxWidth: .infinity)
        } else {
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(viewModel.filteredMemoryItems) { item in
                        MemoryItemRow(item: item) {
                            Task { await viewModel.deleteMemoryItem(id: item.id) }
                        }
                        Divider().padding(.leading, 10)
                    }
                }
            }
        }
    }
}

private struct MemoryItemRow: View {
    let item: MemoryItem
    let onDelete: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            VStack(alignment: .leading, spacing: 3) {
                Text(item.text)
                    .font(.system(size: 11))
                    .lineLimit(3)
                    .foregroundStyle(.primary)
                HStack(spacing: 4) {
                    kindBadge
                    if let src = item.source, src != "passive" {
                        sourceBadge(src)
                    }
                    Text(item.namespace)
                        .font(.system(size: 9))
                        .foregroundStyle(.tertiary)
                    if item.use_count > 0 {
                        Text("×\(item.use_count)")
                            .font(.system(size: 9))
                            .foregroundStyle(.quaternary)
                    }
                }
                if let conf = item.confidence, let imp = item.importance {
                    HStack(spacing: 6) {
                        Label(String(format: "%.0f%%", conf * 100), systemImage: "checkmark.circle")
                            .font(.system(size: 9))
                            .foregroundStyle(.secondary)
                        Label(String(format: "%.0f%%", imp * 100), systemImage: "star")
                            .font(.system(size: 9))
                            .foregroundStyle(.secondary)
                    }
                }
            }
            Spacer()
            Button(action: onDelete) {
                Image(systemName: "trash")
                    .font(.system(size: 10))
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
    }

    private func sourceBadge(_ src: String) -> some View {
        let (label, color): (String, Color) = switch src {
        case "explicit":   ("expl", .blue)
        case "user_edit":  ("edit", .teal)
        case "import":     ("imp",  .indigo)
        default:           (src,    .gray)
        }
        return Text(label)
            .font(.system(size: 9, weight: .medium))
            .padding(.horizontal, 4)
            .padding(.vertical, 2)
            .background(color.opacity(0.15))
            .foregroundStyle(color)
            .clipShape(Capsule())
    }

    private var kindBadge: some View {
        Text(kindLabel)
            .font(.system(size: 9, weight: .medium))
            .padding(.horizontal, 5)
            .padding(.vertical, 2)
            .background(kindColor.opacity(0.18))
            .foregroundStyle(kindColor)
            .clipShape(Capsule())
    }

    private var kindLabel: String {
        switch item.kind {
        case "preference":   return "Preferencia"
        case "correction":   return "Oprava"
        case "sk_glossary":  return "Glosár SK"
        case "style_profile":return "Štýl"
        case "fact":         return "Fakt"
        case "entity":       return "Entita"
        case "instruction":  return "Inštrukcia"
        default:             return item.kind
        }
    }

    private var kindColor: Color {
        switch item.kind {
        case "preference":    return .blue
        case "correction":    return .orange
        case "sk_glossary":   return .purple
        case "style_profile": return .green
        case "fact":          return .cyan
        case "entity":        return .indigo
        case "instruction":   return .red
        default:              return .gray
        }
    }
}
