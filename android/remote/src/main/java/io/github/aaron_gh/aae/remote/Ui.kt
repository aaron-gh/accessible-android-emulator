package io.github.aaron_gh.aae.remote

import android.app.Activity
import android.app.AlertDialog
import android.text.InputType
import android.util.TypedValue
import android.view.View
import android.view.ViewGroup
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ListView
import android.widget.ScrollView
import android.widget.TextView

/**
 * Builds screens from standard Android controls, top to bottom: headings,
 * text, buttons, rows of buttons and lists, with a live-region status line.
 */
class Ui(private val activity: Activity) {
    val column = LinearLayout(activity).apply {
        orientation = LinearLayout.VERTICAL
        val pad = dp(16)
        setPadding(pad, pad, pad, pad)
    }
    /** Says what's happening, and screen readers announce each change. */
    val status: TextView = TextView(activity).apply {
        accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        setPadding(0, dp(8), 0, dp(8))
        // Nothing to say yet: hidden, so it isn't read as an empty item.
        visibility = View.GONE
    }

    fun dp(value: Int) = TypedValue.applyDimension(
        TypedValue.COMPLEX_UNIT_DIP, value.toFloat(), activity.resources.displayMetrics,
    ).toInt()

    /** Shows the screen, scrolling unless it holds a list, which scrolls itself. */
    fun show(scroll: Boolean = true) {
        val root = if (scroll) ScrollView(activity).apply { addView(column) } else column
        fitBars(root)
        activity.setContentView(root)
    }

    /** Keeps a view clear of the status and navigation bars, which apps draw under from Android 15. */
    fun fitBars(view: View) {
        val (left, top, right, bottom) = listOf(view.paddingLeft, view.paddingTop, view.paddingRight, view.paddingBottom)
        view.setOnApplyWindowInsetsListener { v, insets ->
            if (android.os.Build.VERSION.SDK_INT >= 30) {
                val bars = insets.getInsets(android.view.WindowInsets.Type.systemBars() or android.view.WindowInsets.Type.displayCutout())
                v.setPadding(left + bars.left, top + bars.top, right + bars.right, bottom + bars.bottom)
            } else {
                @Suppress("DEPRECATION")
                v.setPadding(
                    left + insets.systemWindowInsetLeft, top + insets.systemWindowInsetTop,
                    right + insets.systemWindowInsetRight, bottom + insets.systemWindowInsetBottom,
                )
            }
            insets
        }
    }

    /** A one-line text field with a label above it, which names it for screen readers. */
    fun field(label: String): EditText {
        val title = TextView(activity).apply {
            text = label
            setPadding(0, dp(4), 0, 0)
        }
        column.addView(title)
        return EditText(activity).apply {
            id = View.generateViewId()
            title.labelFor = id
            setSingleLine()
            column.addView(this)
        }
    }

    fun heading(text: String): TextView = TextView(activity).apply {
        this.text = text
        isAccessibilityHeading = true
        setTextSize(TypedValue.COMPLEX_UNIT_SP, 22f)
        setPadding(0, dp(8), 0, dp(8))
        column.addView(this)
    }

    fun text(text: String): TextView = TextView(activity).apply {
        this.text = text
        setPadding(0, dp(4), 0, dp(4))
        column.addView(this)
    }

    fun addStatus() = column.addView(status)

    fun say(text: String) {
        status.visibility = View.VISIBLE
        status.text = text
    }

    /** The row buttons go in, while [row] builds one. */
    private var row: LinearLayout? = null

    fun button(text: String, action: () -> Unit): Button = Button(activity).apply {
        this.text = text
        setOnClickListener { action() }
        val row = row
        if (row != null) {
            row.addView(this, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1f))
        } else {
            column.addView(this)
        }
    }

    /** Puts the buttons [build] adds side by side, sharing the width. */
    fun row(build: () -> Unit) {
        val line = LinearLayout(activity).apply { orientation = LinearLayout.HORIZONTAL }
        column.addView(line)
        row = line
        try {
            build()
        } finally {
            row = null
        }
    }

    /** A list of rows that takes the space left; [chosen] gets the row's index. */
    fun list(label: String, chosen: (Int) -> Unit): Pair<ListView, ArrayAdapter<String>> {
        val adapter = ArrayAdapter<String>(activity, android.R.layout.simple_list_item_1)
        val list = ListView(activity).apply {
            this.adapter = adapter
            contentDescription = label
            setOnItemClickListener { _, _, position, _ -> chosen(position) }
            layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f)
        }
        // An empty list says nothing useful; the status line says why it's empty.
        list.visibility = View.GONE
        adapter.registerDataSetObserver(object : android.database.DataSetObserver() {
            override fun onChanged() {
                list.visibility = if (adapter.isEmpty) View.INVISIBLE else View.VISIBLE
            }
        })
        column.addView(list)
        return list to adapter
    }

    fun message(title: String, text: String, done: () -> Unit = {}) {
        AlertDialog.Builder(activity).setTitle(title).setMessage(text)
            .setPositiveButton(android.R.string.ok) { _, _ -> done() }
            .setOnCancelListener { done() }
            .show()
    }

    /** Asks yes or no. The safe answer is Cancel. */
    fun confirm(title: String, text: String, action: String, yes: () -> Unit) {
        AlertDialog.Builder(activity).setTitle(title).setMessage(text)
            .setNegativeButton(android.R.string.cancel, null)
            .setPositiveButton(action) { _, _ -> yes() }
            .show()
    }

    /** Asks for text. */
    fun ask(title: String, hint: String, initial: String = "", action: String, done: (String) -> Unit) {
        // The hint labels the field. A content description would hide what's typed.
        val field = EditText(activity).apply {
            this.hint = hint
            setText(initial)
            inputType = InputType.TYPE_CLASS_TEXT
            setSingleLine()
        }
        val box = LinearLayout(activity).apply {
            val pad = dp(16)
            setPadding(pad, 0, pad, 0)
            addView(field)
        }
        AlertDialog.Builder(activity).setTitle(title).setView(box)
            .setNegativeButton(android.R.string.cancel, null)
            .setPositiveButton(action) { _, _ -> done(field.text.toString().trim()) }
            .show()
        field.requestFocus()
    }

    /** Chooses one of [items]. */
    fun choose(title: String, items: List<String>, done: (Int) -> Unit) {
        AlertDialog.Builder(activity).setTitle(title)
            .setItems(items.toTypedArray()) { _, which -> done(which) }
            .setNegativeButton(android.R.string.cancel, null)
            .show()
    }

    /** Shows a licence to read, and asks to accept it. */
    fun licence(title: String, text: String, accepted: (Boolean) -> Unit) {
        val body = TextView(activity).apply {
            this.text = text
            val pad = dp(16)
            setPadding(pad, pad, pad, pad)
            setTextIsSelectable(true)
        }
        AlertDialog.Builder(activity).setTitle(title)
            .setView(ScrollView(activity).apply { addView(body) })
            .setNegativeButton("Decline") { _, _ -> accepted(false) }
            .setPositiveButton("Accept") { _, _ -> accepted(true) }
            .setOnCancelListener { accepted(false) }
            .show()
    }
}
