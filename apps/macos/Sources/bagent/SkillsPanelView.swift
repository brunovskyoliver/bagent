import SwiftUI

struct SkillsPanelView: View {
    @ObservedObject var viewModel: ChatViewModel

    var body: some View {
        VStack(spacing: 0) {
            skillList
        }
        .background(.ultraThinMaterial)
    }

    @ViewBuilder
    private var skillList: some View {
        if viewModel.isLoadingSkills {
            VStack {
                Spacer()
                ProgressView().scaleEffect(0.7)
                Text("Načítavam…")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                Spacer()
            }
            .frame(maxWidth: .infinity)
        } else if viewModel.skills.isEmpty {
            VStack(spacing: 6) {
                Spacer()
                Image(systemName: "wand.and.stars")
                    .font(.system(size: 24))
                    .foregroundStyle(.tertiary)
                Text("Žiadne načítané schopnosti.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                Spacer()
            }
            .frame(maxWidth: .infinity)
        } else {
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(viewModel.skills) { skill in
                        SkillRow(skill: skill)
                        Divider().padding(.leading, 10)
                    }
                }
            }
        }
    }
}

private struct SkillRow: View {
    let skill: SkillItem
    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.easeInOut(duration: 0.15)) { expanded.toggle() }
            } label: {
                HStack(alignment: .top, spacing: 8) {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 12)
                        .padding(.top, 1)
                    VStack(alignment: .leading, spacing: 3) {
                        HStack(spacing: 6) {
                            Text(skill.name)
                                .font(.system(size: 11, weight: .medium))
                                .foregroundStyle(.primary)
                            riskBadge
                        }
                        Text(skill.description)
                            .font(.system(size: 10))
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                        if !skill.tags.isEmpty {
                            HStack(spacing: 4) {
                                ForEach(skill.tags.prefix(4), id: \.self) { tag in
                                    Text(tag)
                                        .font(.system(size: 9))
                                        .padding(.horizontal, 4)
                                        .padding(.vertical, 1)
                                        .background(Color.secondary.opacity(0.12))
                                        .foregroundStyle(.secondary)
                                        .clipShape(Capsule())
                                }
                            }
                        }
                    }
                    Spacer(minLength: 4)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if expanded, let body = skill.body, !body.isEmpty {
                ScrollView {
                    Text(body)
                        .font(.system(size: 10, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(8)
                }
                .frame(maxHeight: 160)
                .background(Color.black.opacity(0.06))
                .clipShape(RoundedRectangle(cornerRadius: 6))
                .padding(.horizontal, 10)
                .padding(.bottom, 8)
            }
        }
    }

    private var riskBadge: some View {
        let (label, color): (String, Color) = switch skill.risk {
        case "high":   ("high", .red)
        case "medium": ("med",  .orange)
        default:       ("low",  .green)
        }
        return Text(label)
            .font(.system(size: 9, weight: .medium))
            .padding(.horizontal, 5)
            .padding(.vertical, 2)
            .background(color.opacity(0.18))
            .foregroundStyle(color)
            .clipShape(Capsule())
    }
}
