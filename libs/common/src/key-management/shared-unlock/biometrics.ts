import { uuidAsString } from "@bitwarden/common/platform/abstractions/sdk/sdk.service";
// eslint-disable-next-line no-restricted-imports
import { BiometricsService, BiometricsStatus } from "@bitwarden/key-management";
import { UserId, BiometricsUnlock } from "@bitwarden/sdk-internal";
import { UserId as TSUserId } from "@bitwarden/user-core";

export function createBiometricsDriver(
    biometricsService: BiometricsService
): BiometricsUnlock {
  return {
    get_biometric_available: async (user_id: UserId) => {
      const available = await biometricsService.getBiometricsStatus() === BiometricsStatus.Available;
      return available;
    },
    unlock_biometrics: async (user_id: UserId) => {
      const result = await biometricsService.unlockWithBiometricsForUser(uuidAsString(user_id) as TSUserId);
      console.log("Biometric unlock result", { user_id, result });
      return true;
    },
  };
}