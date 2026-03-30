import { uuidAsString } from "@bitwarden/common/platform/abstractions/sdk/sdk.service";
// eslint-disable-next-line no-restricted-imports
import { BiometricsService, BiometricsStatus, KeyService } from "@bitwarden/key-management";
import { UserId, BiometricsUnlock, BiometricsStatus as SdkBiometricsStatus } from "@bitwarden/sdk-internal";
import { UserId as TSUserId } from "@bitwarden/user-core";

export function createBiometricsDriver(
    biometricsService: BiometricsService,
    keyService: KeyService
): BiometricsUnlock {
  return {
    get_biometrics_status: async (user_id: UserId) => {
      const status = await biometricsService.getBiometricsStatusForUser(uuidAsString(user_id) as TSUserId);
      switch (status) {
        case BiometricsStatus.Available:
          return SdkBiometricsStatus.Available
        case BiometricsStatus.HardwareUnavailable:
          return SdkBiometricsStatus.HardwareUnavailable;
        case BiometricsStatus.NotEnabledLocally:
          return SdkBiometricsStatus.NotEnabled;
        case BiometricsStatus.UnlockNeeded:
          return SdkBiometricsStatus.UnlockNeeded;
      }
    },
    unlock_biometrics: async (user_id: UserId) => {
      const key = await biometricsService.unlockWithBiometricsForUser(uuidAsString(user_id) as TSUserId);
      console.log("Biometrics unlock successful, setting user key");
      await keyService.setUserKey(key, uuidAsString(user_id) as TSUserId);
      return true;
    },
  };
}