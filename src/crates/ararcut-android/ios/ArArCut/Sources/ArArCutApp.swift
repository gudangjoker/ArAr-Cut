// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 ArAr Cut contributors

import AVFoundation
import SwiftUI

enum ArArPalette {
    static let background = Color(red: 0.047, green: 0.051, blue: 0.063)       // #0C0D10
    static let surface = Color(red: 0.098, green: 0.106, blue: 0.125)          // #191B20
    static let surfaceElevated = Color(red: 0.133, green: 0.145, blue: 0.176)  // #22252D
    static let accent = Color(red: 0.0, green: 0.784, blue: 1.0)               // #00C8FF
    static let accentViolet = Color(red: 0.659, green: 0.333, blue: 0.969)     // #A855F7
    static let textPrimary = Color.white
    static let textSecondary = Color(white: 0.66)
    static let danger = Color(red: 0.95, green: 0.28, blue: 0.33)
}

@main
struct ArArCutApp: App {
    @StateObject private var appState = AppStore()

    init() {
        configureAudioSession()
    }

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(appState)
                .preferredColorScheme(.dark)
                .task {
                    appState.bootstrap()
                }
        }
    }

    private func configureAudioSession() {
        let session = AVAudioSession.sharedInstance()
        try? session.setCategory(.playback, mode: .default, options: [.mixWithOthers])
        try? session.setActive(true)
    }
}

struct RootView: View {
    @EnvironmentObject private var app: AppStore

    var body: some View {
        ZStack {
            ArArPalette.background.ignoresSafeArea()
            if let error = app.bootError {
                VStack(spacing: 14) {
                    Image("ArArCutLogo")
                        .resizable()
                        .scaledToFit()
                        .frame(width: 72, height: 72)
                        .clipShape(RoundedRectangle(cornerRadius: 16))
                    Text("ArAr Cut Engine Error")
                        .font(.headline)
                    Text(error)
                        .font(.footnote)
                        .foregroundStyle(ArArPalette.textSecondary)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 28)
                    Button("Retry") { app.bootstrap() }
                        .buttonStyle(.borderedProminent)
                        .tint(ArArPalette.accent)
                }
            } else if let editor = app.activeEditor {
                EditorView(store: editor) {
                    app.closeEditor()
                }
                .transition(.move(edge: .trailing))
            } else if app.engineReady {
                HomeView()
            } else {
                VStack(spacing: 16) {
                    Image("ArArCutLogo")
                        .resizable()
                        .scaledToFit()
                        .frame(width: 84, height: 84)
                        .clipShape(RoundedRectangle(cornerRadius: 20))
                    ProgressView()
                        .tint(ArArPalette.accent)
                    Text("Starting ArAr Cut Engine…")
                        .font(.footnote)
                        .foregroundStyle(ArArPalette.textSecondary)
                }
            }
        }
        .animation(.easeInOut(duration: 0.2), value: app.activeEditor != nil)
    }
}
