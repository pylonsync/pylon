import { useState, useEffect } from "react";
import NetInfo, { type NetInfoState } from "@react-native-community/netinfo";

// ---------------------------------------------------------------------------
// Network status hook
// ---------------------------------------------------------------------------

export interface NetworkStatus {
  /** Whether the device currently has internet connectivity. */
  isOnline: boolean;
  /** Connection type reported by NetInfo (e.g. "wifi", "cellular", "none"). */
  connectionType: string;
}

/**
 * Subscribe to network connectivity changes. Useful for toggling offline
 * banners or pausing sync when the device goes offline.
 *
 * ```tsx
 * const { isOnline, connectionType } = useNetworkStatus();
 * if (!isOnline) return <OfflineBanner />;
 * ```
 */
export function useNetworkStatus(): NetworkStatus {
  const [isOnline, setIsOnline] = useState(true);
  const [connectionType, setConnectionType] = useState<string>("unknown");

  useEffect(() => {
    const handleChange = (state: NetInfoState) => {
      setIsOnline(state.isConnected ?? false);
      setConnectionType(state.type);
    };

    const unsubscribe = NetInfo.addEventListener(handleChange);
    return () => {
      unsubscribe();
    };
  }, []);

  return { isOnline, connectionType };
}
