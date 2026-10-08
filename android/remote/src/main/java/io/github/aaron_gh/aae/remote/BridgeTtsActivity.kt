package io.github.aaron_gh.aae.remote

import android.app.Activity
import android.content.Context
import android.os.Bundle
import android.speech.tts.TextToSpeech
import android.widget.ArrayAdapter
import android.widget.Spinner

/**
 * Bridge Text-to-Speech Settings: engine and rate for the speech bridge.
 * Defaults to the system's.
 */
class BridgeTtsActivity : Activity() {
    private lateinit var ui: Ui
    private var tts: TextToSpeech? = null
    private var engines = listOf<TextToSpeech.EngineInfo>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = Ui(this)
        ui.heading("Bridge Text-to-Speech Settings")
        ui.addStatus()
        val engineList = spinner("Engine", listOf("System default"))
        val rates = listOf(0f) + RATES
        val rateList = spinner("Rate", listOf("System default") + RATES.map { "${(it * 100).toInt()}%" })
        rateList.setSelection(rates.indexOf(RemoteSettings.bridgeRate).coerceAtLeast(0))
        ui.button("Save") {
            val engine = engines.getOrNull(engineList.selectedItemPosition - 1)?.name
            RemoteSettings.setBridge(engine, rates[rateList.selectedItemPosition])
            Remote.voiceChanged()
            ui.say("Saved.")
        }
        ui.button("Try It") {
            val engine = engines.getOrNull(engineList.selectedItemPosition - 1)?.name
            val chosenRate = rates[rateList.selectedItemPosition]
            tts?.shutdown()
            tts = TextToSpeech(this, { status ->
                if (status != TextToSpeech.SUCCESS) return@TextToSpeech ui.say("That engine couldn't start.")
                if (chosenRate > 0f) tts?.setSpeechRate(chosenRate)
                tts?.speak("Speech bridge test.", TextToSpeech.QUEUE_FLUSH, null, "try")
            }, engine)
        }
        ui.show()
        // The engines, once one has started to list them.
        tts = TextToSpeech(this) { _ ->
            engines = tts?.engines.orEmpty().sortedBy { it.label.lowercase() }
            val labels = listOf("System default") + engines.map { it.label }
            engineList.adapter = ArrayAdapter(this, android.R.layout.simple_spinner_item, labels).apply {
                setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
            }
            engineList.setSelection(engines.indexOfFirst { it.name == RemoteSettings.bridgeEngine } + 1)
        }
    }

    override fun onDestroy() {
        tts?.shutdown()
        super.onDestroy()
    }

    private fun spinner(label: String, rows: List<String>): Spinner {
        val labelView = ui.text(label)
        return Spinner(this).apply {
            id = android.view.View.generateViewId()
            labelView.labelFor = id
            adapter = ArrayAdapter(this@BridgeTtsActivity, android.R.layout.simple_spinner_item, rows).apply {
                setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
            }
            ui.column.addView(this)
        }
    }

    companion object {
        val RATES = listOf(0.5f, 0.75f, 1f, 1.25f, 1.5f, 1.75f, 2f, 2.5f, 3f, 3.5f, 4f)
    }
}
