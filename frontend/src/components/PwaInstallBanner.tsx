import React, { useState, useEffect } from 'react';
import { Button } from '../components/ui/button';

interface BeforeInstallPromptEvent extends Event {
  prompt: () => Promise<void>;
  userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>;
}

const PwaInstallBanner: React.FC = () => {
  const [installPrompt, setInstallPrompt] = useState<BeforeInstallPromptEvent | null>(null);
  const [isInstalled, setIsInstalled] = useState(false);

  useEffect(() => {
    // 检查是否已经安装为 PWA
    const checkIfInstalled = () => {
      // 通过 display-mode 媒体查询检测是否以独立应用模式运行
      const isStandalone = window.matchMedia('(display-mode: standalone)').matches ||
        (window.navigator as any).standalone ||
        document.referrer.includes('android-app://');

      setIsInstalled(isStandalone);
    };

    checkIfInstalled();

    // 捕获安装提示事件
    const handleBeforeInstallPrompt = (e: Event) => {
      e.preventDefault();
      setInstallPrompt(e as BeforeInstallPromptEvent);
    };

    // 监听 PWA 安装事件
    window.addEventListener('beforeinstallprompt', handleBeforeInstallPrompt);
    window.addEventListener('appinstalled', () => setIsInstalled(true));

    return () => {
      window.removeEventListener('beforeinstallprompt', handleBeforeInstallPrompt);
      window.removeEventListener('appinstalled', () => setIsInstalled(true));
    };
  }, []);

  const handleInstallClick = async () => {
    if (!installPrompt) return;

    // 显示安装提示
    await installPrompt.prompt();

    // 等待用户响应
    const choiceResult = await installPrompt.userChoice;

    if (choiceResult.outcome === 'accepted') {
      console.log('用户接受了安装');
      setInstallPrompt(null);
    } else {
      console.log('用户拒绝了安装');
    }
  };

  if (isInstalled) {
    return (
      <div className="flex bg-green-50 border border-green-200 rounded-md p-4 mb-4">
        <div className="flex flex-col">
          <div className="flex-shrink-0">
            <svg className="h-5 w-5 text-green-400" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor">
              <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z" clipRule="evenodd" />
            </svg>
          </div>
          <h3 className="text-sm font-medium text-green-800">已安装</h3>
        </div>
        <div className="mt-2 text-sm text-green-700">
          <p>Zhang Accounting 已作为应用安装在您的设备上。</p>
        </div>
      </div>
    );
  }

if (!installPrompt) {
  return (
    <div className="flex flex-col bg-gray-50 border border-gray-200 rounded-md p-4 mb-4 gap-4">
      <div className="flex gap-4">
        <div className="flex-shrink-0">
          <svg className="h-5 w-5 text-gray-400" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor">
            <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z" clipRule="evenodd" />
          </svg>
        </div>
        <h3 className="text-sm font-medium text-gray-800">安装应用</h3>
      </div>


      <div className="mt-2 text-sm text-gray-700">
        <p>您可以在手机上安装 Zhang Accounting 应用，获得更好的离线体验。</p>
        <p className="mt-1">在 iOS 设备上：点击分享按钮，然后选择"添加到主屏幕"。</p>
        <p className="mt-1">在 Android 设备上：点击浏览器菜单，然后选择"添加到主屏幕"。</p>
      </div>
      <div className="mt-4">
        <Button
          onClick={handleInstallClick}
          variant="outline"
          className="text-sm"
        >
          安装应用
        </Button>
      </div>
    </div>);
}

return (
  <div className="bg-blue-50 border border-blue-200 rounded-md p-4 mb-4">
    <div className="flex">
      <div className="flex-shrink-0">
        <svg className="h-5 w-5 text-blue-400" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor">
          <path fillRule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7-4a1 1 0 11-2 0 1 1 0 012 0zM9 9a1 1 0 000 2v3a1 1 0 001 1h1a1 1 0 100-2h-1V9z" clipRule="evenodd" />
        </svg>
      </div>
      <div className="ml-3 flex-1">
        <h3 className="text-sm font-medium text-blue-800">安装应用</h3>
        <div className="mt-2 text-sm text-blue-700">
          <p>将 Zhang Accounting 安装到您的设备上，以获得更好的体验。</p>
        </div>
        <div className="mt-4">
          <Button
            onClick={handleInstallClick}
            variant="outline"
            className="text-sm text-blue-800 bg-blue-100 hover:bg-blue-200"
          >
            安装应用
          </Button>
        </div>
      </div>
    </div>
  </div>
);
};

export default PwaInstallBanner; 