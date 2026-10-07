package io.github.aaron_gh.aae.remote

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.os.Handler
import android.os.Looper

/** A computer AAE is serving from, found on the local network. */
data class Found(val name: String, val host: String, val port: Int, val fingerprint: String)

/**
 * Finds computers running AAE's server on the local network, which announce
 * themselves as `_aae._tcp` with their certificate's fingerprint.
 */
class Discovery(context: Context, private val changed: (List<Found>) -> Unit) {
    private val nsd = context.getSystemService(NsdManager::class.java)
    private val wifi = context.applicationContext.getSystemService(WifiManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private val found = LinkedHashMap<String, Found>()
    private var lock: WifiManager.MulticastLock? = null
    private var running = false

    private val listener = object : NsdManager.DiscoveryListener {
        override fun onDiscoveryStarted(serviceType: String) {}
        override fun onDiscoveryStopped(serviceType: String) {}
        override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {}
        override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}

        override fun onServiceFound(service: NsdServiceInfo) {
            @Suppress("DEPRECATION")
            nsd.resolveService(service, object : NsdManager.ResolveListener {
                override fun onResolveFailed(service: NsdServiceInfo, errorCode: Int) {}

                override fun onServiceResolved(service: NsdServiceInfo) {
                    @Suppress("DEPRECATION")
                    val host = service.host?.hostAddress ?: return
                    val fingerprint = service.attributes["fingerprint"]?.let { String(it) } ?: ""
                    main.post {
                        found[service.serviceName] = Found(service.serviceName, host, service.port, fingerprint)
                        changed(found.values.toList())
                    }
                }
            })
        }

        override fun onServiceLost(service: NsdServiceInfo) {
            main.post {
                if (found.remove(service.serviceName) != null) changed(found.values.toList())
            }
        }
    }

    fun start() {
        if (running) return
        running = true
        lock = wifi?.createMulticastLock("aae")?.apply { acquire() }
        nsd.discoverServices("_aae._tcp", NsdManager.PROTOCOL_DNS_SD, listener)
    }

    fun stop() {
        if (!running) return
        running = false
        try {
            nsd.stopServiceDiscovery(listener)
        } catch (_: Exception) {
        }
        lock?.release()
        lock = null
    }
}
