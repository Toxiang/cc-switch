import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { FullScreenPanel } from "@/components/common/FullScreenPanel";
import type { CustomEndpoint, Provider } from "@/types";
import type { AppId, ManagedAuthProvider } from "@/lib/api";
import {
  ProviderForm,
  type ProviderFormValues,
} from "@/components/providers/forms/ProviderForm";
import { AuthSettingsPanel } from "@/components/providers/AuthSettingsPanel";
import { codexProviderPresets } from "@/config/codexProviderPresets";
import { extractCodexBaseUrl } from "@/utils/providerConfigUtils";

interface AddProviderDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  appId: AppId;
  onSubmit: (
    provider: Omit<Provider, "id"> & { providerKey?: string },
  ) => Promise<void> | void;
}

export function AddProviderDialog({
  open,
  onOpenChange,
  appId,
  onSubmit,
}: AddProviderDialogProps) {
  const { t } = useTranslation();
  const [isFormSubmitting, setIsFormSubmitting] = useState(false);
  const [authSettingsTarget, setAuthSettingsTarget] =
    useState<ManagedAuthProvider | null>(null);

  useEffect(() => {
    setAuthSettingsTarget(null);
  }, [appId, open]);

  const closeDialog = useCallback(() => {
    setAuthSettingsTarget(null);
    onOpenChange(false);
  }, [onOpenChange]);

  const handlePanelClose = useCallback(() => {
    if (authSettingsTarget) {
      setAuthSettingsTarget(null);
      return;
    }
    closeDialog();
  }, [authSettingsTarget, closeDialog]);

  const formReadyToken = useMemo(
    () => Symbol("provider-form-ready"),
    [appId, open],
  );
  const currentFormReadyToken = useRef(formReadyToken);
  currentFormReadyToken.current = formReadyToken;
  const [formReadyState, setFormReadyState] = useState({
    token: formReadyToken,
    ready: true,
  });
  const isFormReady =
    formReadyState.token === formReadyToken ? formReadyState.ready : true;
  const handleSubmitReadyChange = useCallback(
    (ready: boolean) => {
      if (currentFormReadyToken.current === formReadyToken) {
        setFormReadyState({ token: formReadyToken, ready });
      }
    },
    [formReadyToken],
  );

  const handleSubmit = useCallback(
    async (values: ProviderFormValues) => {
      const parsedConfig = JSON.parse(values.settingsConfig) as Record<
        string,
        unknown
      >;
      const providerData: Omit<Provider, "id"> & { providerKey?: string } = {
        name: values.name.trim(),
        notes: values.notes?.trim() || undefined,
        websiteUrl: values.websiteUrl?.trim() || undefined,
        settingsConfig: parsedConfig,
        icon: values.icon?.trim() || undefined,
        iconColor: values.iconColor?.trim() || undefined,
        ...(values.presetCategory ? { category: values.presetCategory } : {}),
        ...(values.meta ? { meta: values.meta } : {}),
      };

      if (appId === "opencode" && values.providerKey) {
        providerData.providerKey = values.providerKey;
      }

      const hasCustomEndpoints =
        providerData.meta?.custom_endpoints &&
        Object.keys(providerData.meta.custom_endpoints).length > 0;

      if (!hasCustomEndpoints && values.presetCategory !== "omo") {
        const urlSet = new Set<string>();
        const addUrl = (rawUrl?: string) => {
          const url = (rawUrl || "").trim().replace(/\/+$/, "");
          if (url.startsWith("http")) urlSet.add(url);
        };

        if (appId === "codex" && values.presetId) {
          const presetIndex = Number.parseInt(
            values.presetId.replace("codex-", ""),
            10,
          );
          const preset = codexProviderPresets[presetIndex];
          preset?.endpointCandidates?.forEach(addUrl);
        }

        if (appId === "codex") {
          const config = parsedConfig.config as string | undefined;
          if (config) addUrl(extractCodexBaseUrl(config));
        } else if (appId === "opencode") {
          const options = parsedConfig.options as
            | Record<string, unknown>
            | undefined;
          if (typeof options?.baseURL === "string") addUrl(options.baseURL);
        }

        if (urlSet.size > 0) {
          const now = Date.now();
          const customEndpoints: Record<string, CustomEndpoint> = {};
          for (const url of urlSet) {
            customEndpoints[url] = { url, addedAt: now, lastUsed: undefined };
          }
          providerData.meta = {
            ...(providerData.meta ?? {}),
            custom_endpoints: customEndpoints,
          };
        }
      }

      await onSubmit(providerData);
      closeDialog();
    },
    [appId, closeDialog, onSubmit],
  );

  return (
    <FullScreenPanel
      isOpen={open}
      title={t("provider.addNewProvider")}
      onClose={handlePanelClose}
      contentClassName="pt-3"
      footer={
        <>
          <span className="mr-auto min-w-0 truncate text-xs text-muted-foreground">
            {t("provider.addFooterHint")}
          </span>
          <Button variant="outline" onClick={closeDialog}>
            {t("common.cancel")}
          </Button>
          <Button
            type="submit"
            form="provider-form"
            disabled={isFormSubmitting || !isFormReady}
          >
            {isFormSubmitting ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : (
              <Plus className="mr-2 h-4 w-4" />
            )}
            {t("common.add")}
          </Button>
        </>
      }
    >
      <ProviderForm
        appId={appId}
        submitLabel={t("common.add")}
        onSubmit={handleSubmit}
        onCancel={closeDialog}
        onManageAuthAccounts={setAuthSettingsTarget}
        onSubmittingChange={setIsFormSubmitting}
        onSubmitReadyChange={handleSubmitReadyChange}
        showButtons={false}
      />
      <AuthSettingsPanel
        target={authSettingsTarget}
        onClose={() => setAuthSettingsTarget(null)}
      />
    </FullScreenPanel>
  );
}
