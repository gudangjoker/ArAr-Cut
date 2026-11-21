# UniFFI's Kotlin bindings reach the Rust library through JNA, which finds the bindings' native
# functions and structures by reflection: keep both.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class com.ararcut.app.core.** { *; }
# The JNI entry point the native library names: Java_com_ararcut_app_NativeContext_init
-keep class com.ararcut.app.NativeContext { native <methods>; }
-dontwarn java.awt.**
