package io.github.aaron_gh.aae.helper

import android.content.res.Configuration
import android.os.LocaleList

/**
 * The device's languages, as Settings sets them: "locale" prints them as
 * language tags, such as "fr-FR,en-US", and "locale fr-FR,en-US" sets them.
 *
 * Settings changes them through the activity manager's hidden interface,
 * which needs the CHANGE_CONFIGURATION permission. The shell user has it.
 */
object SystemLocale {
    fun run(tags: String?) {
        val am = Class.forName("android.app.ActivityManager").getMethod("getService").invoke(null)!!
        if (tags == null) {
            val config = am.javaClass.getMethod("getConfiguration").invoke(am) as Configuration
            println(config.locales.toLanguageTags())
            return
        }
        val locales = LocaleList.forLanguageTags(tags)
        if (locales.isEmpty) throw IllegalArgumentException("no languages in \"$tags\"")
        val config = Configuration()
        config.setLocales(locales)
        // Marks it as the user's choice, as Settings does, so it's kept.
        Configuration::class.java.getField("userSetLocale").setBoolean(config, true)
        val withAttribution = am.javaClass.methods.firstOrNull {
            it.name == "updatePersistentConfigurationWithAttribution" && it.parameterTypes.size == 3
        }
        if (withAttribution != null) {
            withAttribution.invoke(am, config, "com.android.shell", null)
        } else {
            am.javaClass.getMethod("updatePersistentConfiguration", Configuration::class.java).invoke(am, config)
        }
        println(locales.toLanguageTags())
    }
}
