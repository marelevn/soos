package io.github.marelevn.soos

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier

class MainActivity : ComponentActivity() {
    private val state: AppState by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent { SoosScreen(state) }
    }

    override fun onResume() {
        super.onResume()
        state.refreshRates()
    }

    // May be the last the process hears of it, so the tabs are written now.
    override fun onStop() {
        state.saveNow()
        super.onStop()
    }
}

@Composable
fun SoosScreen(state: AppState) {
    val palette = palette()
    BackHandler(state.showConverters) { state.showConverters = false }
    Column(
        Modifier
            .fillMaxSize()
            .background(palette.background)
            // The tab strip handles the top itself, so its colour reaches behind the status bar.
            .windowInsetsPadding(
                WindowInsets.safeDrawing.only(WindowInsetsSides.Horizontal + WindowInsetsSides.Bottom),
            ),
    ) {
        TabStrip(state, palette)
        Box(Modifier.weight(1f)) {
            if (state.showConverters) {
                ConvertersScreen(state, palette)
            } else {
                Document(state, palette)
                Column(Modifier.align(Alignment.BottomCenter)) {
                    Toast(state, palette)
                    UndoBar(state, palette)
                }
            }
        }
        StatusBar(state, palette)
    }
}
