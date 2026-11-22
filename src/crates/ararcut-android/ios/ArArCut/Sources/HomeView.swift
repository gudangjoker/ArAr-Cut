// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 ArAr Cut contributors

import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

struct TransferableMedia: Transferable {
    let url: URL

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(contentType: .movie) { movie in
            SentTransferredFile(movie.url)
        } importing: { received in
            let copy = FileManager.default.temporaryDirectory
                .appendingPathComponent("\(UUID().uuidString)-\(received.file.lastPathComponent)")
            try FileManager.default.copyItem(at: received.file, to: copy)
            return Self(url: copy)
        }
        FileRepresentation(contentType: .image) { image in
            SentTransferredFile(image.url)
        } importing: { received in
            let copy = FileManager.default.temporaryDirectory
                .appendingPathComponent("\(UUID().uuidString)-\(received.file.lastPathComponent)")
            try FileManager.default.copyItem(at: received.file, to: copy)
            return Self(url: copy)
        }
    }
}

struct HomeView: View {
    @EnvironmentObject private var app: AppStore

    @State private var pickerItems: [PhotosPickerItem] = []
    @State private var showNewBlankSheet = false
    @State private var showAboutSheet = false
    @State private var newProjectName = "Untitled"
    @State private var newAspect = 0 // 0 = 9:16, 1 = 16:9, 2 = 1:1
    @State private var renamingProject: ProjectSummary?
    @State private var renameText = ""

    private let columns = [GridItem(.adaptive(minimum: 165), spacing: 12)]

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    heroHeader
                    actionCards
                    projectsSection
                }
                .padding(16)
            }
            .background(ArArPalette.background.ignoresSafeArea())
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        showAboutSheet = true
                    } label: {
                        Image(systemName: "info.circle")
                            .foregroundStyle(ArArPalette.textSecondary)
                    }
                }
            }
            .overlay {
                if let msg = app.busyMessage {
                    ZStack {
                        Color.black.opacity(0.6).ignoresSafeArea()
                        VStack(spacing: 12) {
                            ProgressView().tint(ArArPalette.accent)
                            Text(msg).font(.subheadline)
                        }
                        .padding(24)
                        .background(ArArPalette.surface)
                        .clipShape(RoundedRectangle(cornerRadius: 14))
                    }
                }
            }
            .alert("ArAr Cut", isPresented: Binding(
                get: { app.toast != nil },
                set: { if !$0 { app.toast = nil } }
            )) {
                Button("OK", role: .cancel) { app.toast = nil }
            } message: {
                Text(app.toast ?? "")
            }
            .alert("Rename Project", isPresented: Binding(
                get: { renamingProject != nil },
                set: { if !$0 { renamingProject = nil } }
            )) {
                TextField("Project name", text: $renameText)
                Button("Cancel", role: .cancel) { renamingProject = nil }
                Button("Save") {
                    if let p = renamingProject { app.rename(p, to: renameText) }
                    renamingProject = nil
                }
            }
            .sheet(isPresented: $showNewBlankSheet) {
                newBlankProjectSheet
            }
            .sheet(isPresented: $showAboutSheet) {
                aboutSheet
            }
            .onChange(of: pickerItems) { items in
                guard !items.isEmpty else { return }
                let chosen = items
                pickerItems = []
                Task {
                    var urls: [URL] = []
                    for item in chosen {
                        if let media = try? await item.loadTransferable(type: TransferableMedia.self) {
                            urls.append(media.url)
                        }
                    }
                    let df = DateFormatter()
                    df.dateFormat = "MMM d HH:mm"
                    app.createAndOpenProject(
                        name: "Edit \(df.string(from: Date()))",
                        width: 1080,
                        height: 1920,
                        fps: 30,
                        initialMedia: urls
                    )
                }
            }
        }
    }

    private var heroHeader: some View {
        HStack(spacing: 12) {
            Image("ArArCutLogo")
                .resizable()
                .scaledToFit()
                .frame(width: 44, height: 44)
                .clipShape(RoundedRectangle(cornerRadius: 11))
            VStack(alignment: .leading, spacing: 2) {
                Text("ArAr Cut")
                    .font(.title2.bold())
                    .foregroundStyle(ArArPalette.textPrimary)
                Text("Pro Offline Video Studio · Rust + Metal")
                    .font(.caption)
                    .foregroundStyle(ArArPalette.textSecondary)
            }
            Spacer()
        }
    }

    private var actionCards: some View {
        HStack(spacing: 12) {
            PhotosPicker(
                selection: $pickerItems,
                maxSelectionCount: 20,
                matching: .any(of: [.videos, .images])
            ) {
                VStack(spacing: 8) {
                    Image(systemName: "plus.circle.fill")
                        .font(.system(size: 30))
                    Text("New Project")
                        .font(.headline)
                    Text("Pick videos & photos")
                        .font(.caption)
                        .opacity(0.85)
                }
                .foregroundStyle(.black)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 22)
                .background(
                    LinearGradient(
                        colors: [ArArPalette.accent, ArArPalette.accentViolet],
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    )
                )
                .clipShape(RoundedRectangle(cornerRadius: 16))
            }

            Button {
                newProjectName = "Project \(app.projects.count + 1)"
                showNewBlankSheet = true
            } label: {
                VStack(spacing: 8) {
                    Image(systemName: "film.stack")
                        .font(.system(size: 26))
                        .foregroundStyle(ArArPalette.accent)
                    Text("Blank Canvas")
                        .font(.headline)
                        .foregroundStyle(ArArPalette.textPrimary)
                    Text("9:16 · 16:9 · 1:1")
                        .font(.caption)
                        .foregroundStyle(ArArPalette.textSecondary)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 22)
                .background(ArArPalette.surface)
                .clipShape(RoundedRectangle(cornerRadius: 16))
                .overlay(
                    RoundedRectangle(cornerRadius: 16)
                        .stroke(Color.white.opacity(0.08), lineWidth: 1)
                )
            }
        }
    }

    private var projectsSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Projects")
                    .font(.headline)
                Spacer()
                Text("\(app.projects.count)")
                    .font(.caption)
                    .foregroundStyle(ArArPalette.textSecondary)
            }

            if app.projects.isEmpty {
                VStack(spacing: 10) {
                    Image(systemName: "sparkles.tv")
                        .font(.largeTitle)
                        .foregroundStyle(ArArPalette.textSecondary)
                    Text("No projects yet")
                        .font(.subheadline.weight(.medium))
                    Text("Tap New Project to start editing with 201 GPU shaders, EBU R128 mastering, Camera Log LUTs, and Live Scopes.")
                        .font(.caption)
                        .foregroundStyle(ArArPalette.textSecondary)
                        .multilineTextAlignment(.center)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 40)
                .padding(.horizontal, 20)
                .background(ArArPalette.surface)
                .clipShape(RoundedRectangle(cornerRadius: 16))
            } else {
                LazyVGrid(columns: columns, spacing: 12) {
                    ForEach(app.projects, id: \.path) { project in
                        projectCard(project)
                    }
                }
            }
        }
    }

    private func projectCard(_ project: ProjectSummary) -> some View {
        Button {
            app.openProject(project)
        } label: {
            VStack(alignment: .leading, spacing: 8) {
                ZStack(alignment: .bottomTrailing) {
                    RoundedRectangle(cornerRadius: 10)
                        .fill(ArArPalette.surfaceElevated)
                        .frame(height: 104)
                        .overlay {
                            Image(systemName: "play.rectangle.fill")
                                .font(.title)
                                .foregroundStyle(ArArPalette.accent.opacity(0.8))
                        }
                    Text(formatTimecode(project.durationUs))
                        .font(.caption2.monospacedDigit().weight(.semibold))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 3)
                        .background(Color.black.opacity(0.72))
                        .clipShape(RoundedRectangle(cornerRadius: 5))
                        .padding(6)
                }
                Text(project.name)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(ArArPalette.textPrimary)
                    .lineLimit(1)
                HStack {
                    Text("\(project.width)×\(project.height)")
                    Spacer()
                    Text(formatBytes(project.sizeBytes))
                }
                .font(.caption2)
                .foregroundStyle(ArArPalette.textSecondary)
            }
            .padding(10)
            .background(ArArPalette.surface)
            .clipShape(RoundedRectangle(cornerRadius: 14))
        }
        .contextMenu {
            Button("Rename", systemImage: "pencil") {
                renameText = project.name
                renamingProject = project
            }
            Button("Duplicate", systemImage: "plus.square.on.square") {
                app.duplicate(project)
            }
            Button("Delete", systemImage: "trash", role: .destructive) {
                app.delete(project)
            }
        }
    }

    private var newBlankProjectSheet: some View {
        NavigationStack {
            Form {
                Section("Project Name") {
                    TextField("Name", text: $newProjectName)
                }
                Section("Aspect Ratio") {
                    Picker("Canvas", selection: $newAspect) {
                        Text("9:16 Vertical (1080×1920)").tag(0)
                        Text("16:9 Landscape (1920×1080)").tag(1)
                        Text("1:1 Square (1080×1080)").tag(2)
                    }
                    .pickerStyle(.inline)
                }
            }
            .navigationTitle("New Blank Project")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { showNewBlankSheet = false }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Create") {
                        let (w, h): (UInt32, UInt32) = switch newAspect {
                        case 1: (1920, 1080)
                        case 2: (1080, 1080)
                        default: (1080, 1920)
                        }
                        showNewBlankSheet = false
                        app.createAndOpenProject(
                            name: newProjectName.isEmpty ? "Untitled" : newProjectName,
                            width: w,
                            height: h,
                            fps: 30
                        )
                    }
                }
            }
        }
        .presentationDetents([.medium])
    }

    private var aboutSheet: some View {
        NavigationStack {
            List {
                Section {
                    HStack(spacing: 14) {
                        Image("ArArCutLogo")
                            .resizable()
                            .scaledToFit()
                            .frame(width: 56, height: 56)
                            .clipShape(RoundedRectangle(cornerRadius: 14))
                        VStack(alignment: .leading, spacing: 3) {
                            Text("ArAr Cut for iOS")
                                .font(.headline)
                            Text("Engine v\(engineVersion()) · Rust + wgpu Metal + FFmpeg 8.1")
                                .font(.caption)
                                .foregroundStyle(ArArPalette.textSecondary)
                        }
                    }
                    .padding(.vertical, 4)
                }
                Section("Pro Modules Included") {
                    Label("EBU R128 Loudness, 6 Mastering Chains & GCC-PHAT Sync", systemImage: "waveform")
                    Label("12 Camera Log → Rec.709 33³ GPU 3D LUTs", systemImage: "camera.filters")
                    Label("Live Waveform, RGB Parade, Vectorscope & Histogram", systemImage: "chart.xyaxis.line")
                    Label("On-Device MFCC Speaker Diarization", systemImage: "person.2.wave.2")
                    Label("Premiere XML, FCPXML 1.11, OTIO & CMX 3600 EDL", systemImage: "square.and.arrow.up")
                }
                Section("Open Source License") {
                    Text("Licensed under AGPL-3.0-or-later. Built on Concat/CapCraft with MIT/Apache-2.0 pro engine modules from FilmCraft.")
                        .font(.caption)
                        .foregroundStyle(ArArPalette.textSecondary)
                    Link("GitHub Repository", destination: URL(string: "https://github.com/gudangjoker/ArAr-Cut")!)
                }
            }
            .navigationTitle("About ArAr Cut")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { showAboutSheet = false }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }
}
