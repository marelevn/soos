package io.github.marelevn.soos

import android.app.Application
import java.io.File

class SoosApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        Engine.init(File(filesDir, "rates.json").path)
    }
}
