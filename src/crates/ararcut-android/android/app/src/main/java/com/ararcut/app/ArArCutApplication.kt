package com.ararcut.app

import android.app.Application
import android.content.ComponentCallbacks2
import android.system.Os
import com.ararcut.app.core.init
import com.ararcut.app.core.trimMemory
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.launch

class ArArCutApplication : Application() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    override fun onCreate() {
        super.onCreate()
        // Rust's temporary folder and home on Android are not the app's own: point them at
        // it synchronously on the main thread before any native/worker thread reads the environment.
        runCatching {
            Os.setenv("TMPDIR", cacheDir.absolutePath, true)
            Os.setenv("HOME", filesDir.absolutePath, true)
        }
    }

    /** The engine, started once per process on a background thread. */
    val engine: Deferred<Result<Unit>> by lazy {
        scope.async {
            runCatching {
                File(cacheDir, "exports").mkdirs()
                init(filesDir.absolutePath, cacheDir.absolutePath)
                NativeContext.start(this@ArArCutApplication)
                engineLoaded = true
            }
        }
    }

    @Volatile
    private var engineLoaded = false

    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        if (level >= ComponentCallbacks2.TRIM_MEMORY_UI_HIDDEN && engineLoaded) {
            scope.launch { runCatching { trimMemory() } }
        }
    }
}
