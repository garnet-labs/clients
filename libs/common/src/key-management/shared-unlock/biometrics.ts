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
    async get_biometrics_status(user_id: UserId): Promise<SdkBiometricsStatus> {
      console.log("Getting biometrics status for user", user_id);
      const status = await biometricsService.getBiometricsStatusForUser(uuidAsString(user_id) as TSUserId);
      console.log("Biometrics status for user", user_id, "is", status);
      switch (status) {
        case BiometricsStatus.Available:
          console.log("returning available for user", user_id);
          return SdkBiometricsStatus.Available;
        case BiometricsStatus.HardwareUnavailable:
          return SdkBiometricsStatus.HardwareUnavailable;
        case BiometricsStatus.NotEnabledLocally:
          return SdkBiometricsStatus.NotEnabled;
        case BiometricsStatus.UnlockNeeded:
          return SdkBiometricsStatus.UnlockNeeded;
        default:
          return SdkBiometricsStatus.NotEnabled;
      }
    },
    async unlock_biometrics(user_id: UserId) {
      const key = await biometricsService.unlockWithBiometricsForUser(uuidAsString(user_id) as TSUserId);
      console.log("Biometrics unlock successful, setting user key");
      await keyService.setUserKey(key, uuidAsString(user_id) as TSUserId);
      return true;
    },
  };
}